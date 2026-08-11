#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const root = resolve(scriptDirectory, "..");
const supportDirectory = resolve(root, "tests/support");
const providerFiles = [
  resolve(supportDirectory, "runtime_completion_oracle.rs"),
  resolve(supportDirectory, "runtime_observation.rs"),
];

const options = parseArguments(process.argv.slice(2));

// These are exact helper families removed by the runtime-observation cleanup.
// Keeping the source, origin, and replacement together makes each tombstone
// reviewable instead of turning the gate into an unexplained hash list.
const tombstoneSources = [
  {
    label: "legacy Error-context completion observer",
    origin: "tests/oracle/arguments/oracle_arguments.rs",
    replacement:
      "the matching runtime_completion_oracle::observe_*_eval_completion helper",
    source: String.raw`fn observe_rust_eval(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    match context.eval(source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            primitive_value_text(value)
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take Rust exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}|{}",
                    error_string_property(runtime, context, &error, "name", description),
                    error_string_property(runtime, context, &error, "message", description),
                ),
                value => format!(
                    "throw|{}|{}",
                    value_type(runtime, &value),
                    primitive_value_text(value)
                ),
            }
        }
        Err(error) => panic!("Rust engine failure for {description} ({source:?}): {error}"),
    }
}`,
  },
  {
    label: "compact completion observer",
    origin: "tests/oracle/object/oracle_object_descriptors.rs",
    replacement: "runtime_completion_oracle::observe_eval_completion",
    source: String.raw`fn observe_rust_eval(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    match context.eval(source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            primitive_value_text(value)
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take Rust exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}|{}",
                    string_property(runtime, context, &error, "name"),
                    string_property(runtime, context, &error, "message"),
                ),
                value => format!(
                    "throw|{}|{}",
                    value_type(runtime, &value),
                    primitive_value_text(value)
                ),
            }
        }
        Err(error) => panic!("Rust engine failure for {description} ({source:?}): {error}"),
    }
}`,
  },
  {
    label: "prelude fail-fast completion comparison",
    origin: "tests/oracle/string/oracle_string_case.rs",
    replacement:
      "runtime_completion_oracle::compare_eval_completion_cases_with_prelude",
    source: String.raw`fn compare_cases(group: &str, cases: &[(&str, &str)]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group}: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in cases {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let source = format!("{CASE_PRELUDE}\n{source}");
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, &source, description),
            observe_oracle_source(&oracle, &source, description),
            "{group} drifted for {description}",
        );
    }
}`,
  },
  {
    label: "description-only compact completion observer",
    origin: "tests/oracle/string/oracle_string_case.rs",
    replacement:
      "runtime_completion_oracle::compare_eval_completion_cases_with_prelude",
    source: String.raw`fn observe_rust_eval(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    match context.eval(source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            primitive_value_text(value),
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take Rust exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}|{}",
                    string_property(runtime, context, &error, "name"),
                    string_property(runtime, context, &error, "message"),
                ),
                value => format!(
                    "throw|{}|{}",
                    value_type(runtime, &value),
                    primitive_value_text(value),
                ),
            }
        }
        Err(error) => panic!("Rust engine failure for {description}: {error}"),
    }
}`,
  },
];

// All exact copies of the shared providers were retired with this gate. Keep
// the mechanism explicit so a narrowly justified exception remains a reviewed
// path-and-fingerprint decision instead of a global clone-count waiver.
const providerAllowlist = new Set();

runCanaries();

const providers = loadProviders();
const tombstones = loadTombstones();
const consumerFiles = collectConsumerFiles();
const functions = consumerFiles.flatMap((path) => scanFile(path));
const failures = checkFunctions(functions, providers, tombstones);

if (options.report) {
  printCensus(functions, consumerFiles, providers, tombstones);
}

if (failures.length > 0) {
  for (const failure of failures) {
    console.error(`error: ${failure}`);
  }
  process.exit(1);
}

console.log(
  `Oracle helper duplication gate checked ${functions.length} non-test functions ` +
    `across ${consumerFiles.length} files.`,
);

function parseArguments(arguments_) {
  const parsed = { report: false, scans: [] };
  for (let index = 0; index < arguments_.length; index += 1) {
    const argument = arguments_[index];
    if (argument === "--report") {
      parsed.report = true;
    } else if (argument === "--scan") {
      const path = arguments_[index + 1];
      if (path === undefined) {
        fail("--scan requires a Rust source path");
      }
      parsed.scans.push(resolve(path));
      index += 1;
    } else {
      fail(
        `usage: ${basename(process.argv[1])} [--report] [--scan RUST_SOURCE]`,
      );
    }
  }
  return parsed;
}

function collectConsumerFiles() {
  const tests = resolve(root, "tests");
  const paths = [];
  for (const entry of readdirSync(tests, { withFileTypes: true })) {
    if (
      entry.isFile() &&
      entry.name.startsWith("oracle_") &&
      entry.name.endsWith(".rs")
    ) {
      paths.push(resolve(tests, entry.name));
    }
  }
  walkRustFiles(resolve(tests, "oracle"), paths);
  for (const path of options.scans) {
    if (!statSync(path).isFile()) {
      fail(`--scan path is not a file: ${path}`);
    }
    paths.push(path);
  }
  return [...new Set(paths)].sort();
}

function walkRustFiles(directory, output) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) {
      walkRustFiles(path, output);
    } else if (entry.isFile() && entry.name.endsWith(".rs")) {
      output.push(path);
    }
  }
}

function loadProviders() {
  const byFingerprint = new Map();
  for (const path of providerFiles) {
    const source = readFileSync(path, "utf8");
    for (const helper of extractFunctions(source, path)) {
      if (!helper.publicCrate) continue;
      const current = byFingerprint.get(helper.fingerprint) ?? [];
      current.push({ name: helper.name, path });
      byFingerprint.set(helper.fingerprint, current);
    }
  }
  return byFingerprint;
}

function loadTombstones() {
  const byFingerprint = new Map();
  for (const tombstone of tombstoneSources) {
    const helpers = extractFunctions(tombstone.source, tombstone.origin);
    if (helpers.length !== 1) {
      fail(`tombstone ${tombstone.label} must contain exactly one function`);
    }
    const fingerprint = helpers[0].fingerprint;
    const previous = byFingerprint.get(fingerprint);
    if (previous !== undefined) {
      fail(
        `tombstones ${previous.label} and ${tombstone.label} have the same fingerprint`,
      );
    }
    byFingerprint.set(fingerprint, tombstone);
  }
  return byFingerprint;
}

function scanFile(path) {
  const source = readFileSync(path, "utf8");
  return extractFunctions(source, path).filter((helper) => !helper.test);
}

function checkFunctions(functions, providers, tombstones) {
  const failures = [];
  const usedAllowlist = new Set();
  for (const helper of functions) {
    const displayPath = display(helper.path);
    const providersForFingerprint = providers.get(helper.fingerprint);
    if (providersForFingerprint !== undefined) {
      const allowlistKey = `${displayPath}\0${helper.fingerprint}`;
      if (providerAllowlist.has(allowlistKey)) {
        usedAllowlist.add(allowlistKey);
      } else {
        const providerNames = providersForFingerprint
          .map((provider) => provider.name)
          .sort()
          .join(", ");
        failures.push(
          `${displayPath}:${helper.line} duplicates shared provider ${providerNames}; ` +
            `import it from tests/support instead ` +
            `(fingerprint ${helper.fingerprint})`,
        );
      }
    }
    const tombstone = tombstones.get(helper.fingerprint);
    if (tombstone !== undefined) {
      failures.push(
        `${displayPath}:${helper.line} restores retired ${tombstone.label} ` +
          `(from ${tombstone.origin}); use ${tombstone.replacement}`,
      );
    }
  }
  for (const allowlistKey of providerAllowlist) {
    if (!usedAllowlist.has(allowlistKey)) {
      failures.push(
        `stale shared-provider allowlist entry: ${allowlistKey.replace("\0", " ")}`,
      );
    }
  }
  return failures.sort();
}

function printCensus(functions, consumerFiles, providers, tombstones) {
  const groups = new Map();
  for (const helper of functions) {
    const group = groups.get(helper.fingerprint) ?? [];
    group.push(helper);
    groups.set(helper.fingerprint, group);
  }
  const duplicates = [...groups.entries()]
    .filter(([, group]) => group.length > 1)
    .sort(
      ([leftHash, left], [rightHash, right]) =>
        right.length - left.length || leftHash.localeCompare(rightHash),
    );
  const instances = duplicates.reduce((sum, [, group]) => sum + group.length, 0);
  console.log(
    `census scanned_files=${consumerFiles.length} ` +
      `files_with_helpers=${new Set(functions.map((helper) => helper.path)).size} ` +
      `functions=${functions.length} groups=${duplicates.length} ` +
      `instances=${instances} redundant=${instances - duplicates.length}`,
  );
  for (const [fingerprint, group] of duplicates) {
    const names = [...new Set(group.map((helper) => helper.name))].sort();
    const locations = group
      .map((helper) => `${display(helper.path)}:${helper.line}`)
      .sort();
    console.log(
      `${group.length}\t${fingerprint.slice(0, 12)}\t${names.join(",")}\t${locations.join(" ")}`,
    );
  }
  console.log(
    `protected providers=${providers.size} tombstones=${tombstones.size}`,
  );
}

function extractFunctions(source, path) {
  const tokens = tokenize(source);
  const functions = [];
  for (let index = 0; index < tokens.length - 1; index += 1) {
    if (tokens[index].text !== "fn" || tokens[index + 1].kind !== "identifier") {
      continue;
    }
    let openIndex = index + 2;
    while (
      openIndex < tokens.length &&
      tokens[openIndex].text !== "{" &&
      tokens[openIndex].text !== ";"
    ) {
      openIndex += 1;
    }
    if (openIndex === tokens.length || tokens[openIndex].text === ";") {
      continue;
    }
    const closeIndex = matchingBrace(tokens, openIndex);
    const containingFunction = functions.find(
      (helper) => index > helper.openIndex && index < helper.closeIndex,
    );
    if (containingFunction !== undefined) {
      index = closeIndex;
      continue;
    }
    if (insideItem(tokens, index, "impl") || insideItem(tokens, index, "trait")) {
      index = closeIndex;
      continue;
    }
    const name = tokens[index + 1].text;
    const canonical = tokens
      .slice(index, closeIndex + 1)
      .map((token, tokenIndex) => (tokenIndex === 1 ? "$NAME" : token.text));
    const lineStart = source.lastIndexOf("\n", tokens[index].start - 1) + 1;
    const prefix = source.slice(lineStart, tokens[index].start);
    functions.push({
      canonical,
      closeIndex,
      fingerprint: fingerprint(canonical),
      line: tokens[index].line,
      name,
      openIndex,
      path,
      publicCrate: /pub\s*\(\s*crate\s*\)\s*$/.test(prefix),
      test: hasTestAttribute(tokens, index),
    });
    index = closeIndex;
  }
  return functions;
}

function insideItem(tokens, tokenIndex, keyword) {
  for (let index = 0; index < tokenIndex; index += 1) {
    if (tokens[index].text !== keyword) continue;
    let openIndex = index + 1;
    while (
      openIndex < tokenIndex &&
      tokens[openIndex].text !== "{" &&
      tokens[openIndex].text !== ";"
    ) {
      openIndex += 1;
    }
    if (tokens[openIndex]?.text !== "{") continue;
    const closeIndex = matchingBrace(tokens, openIndex);
    if (tokenIndex > openIndex && tokenIndex < closeIndex) return true;
    index = closeIndex;
  }
  return false;
}

function hasTestAttribute(tokens, functionIndex) {
  let start = functionIndex - 1;
  while (start >= 0 && !["{", "}", ";"].includes(tokens[start].text)) {
    start -= 1;
  }
  for (let index = start + 1; index + 3 < functionIndex; index += 1) {
    if (
      tokens[index].text === "#" &&
      tokens[index + 1].text === "[" &&
      tokens[index + 2].text === "test" &&
      tokens[index + 3].text === "]"
    ) {
      return true;
    }
  }
  return false;
}

function matchingBrace(tokens, openIndex) {
  let depth = 0;
  for (let index = openIndex; index < tokens.length; index += 1) {
    if (tokens[index].text === "{") depth += 1;
    if (tokens[index].text === "}") depth -= 1;
    if (depth === 0) return index;
  }
  fail(`unterminated function body at line ${tokens[openIndex].line}`);
}

function tokenize(source) {
  const tokens = [];
  let index = 0;
  let line = 1;
  while (index < source.length) {
    const character = source[index];
    const next = source[index + 1];
    if (/\s/u.test(character)) {
      if (character === "\n") line += 1;
      index += 1;
      continue;
    }
    if (character === "/" && next === "/") {
      index += 2;
      while (index < source.length && source[index] !== "\n") index += 1;
      continue;
    }
    if (character === "/" && next === "*") {
      index += 2;
      let depth = 1;
      while (index < source.length && depth > 0) {
        if (source[index] === "\n") line += 1;
        if (source[index] === "/" && source[index + 1] === "*") {
          depth += 1;
          index += 2;
        } else if (source[index] === "*" && source[index + 1] === "/") {
          depth -= 1;
          index += 2;
        } else {
          index += 1;
        }
      }
      if (depth !== 0) fail("unterminated block comment");
      continue;
    }
    const rawEnd = rawStringEnd(source, index);
    if (rawEnd !== undefined) {
      const text = source.slice(index, rawEnd);
      tokens.push({ kind: "literal", line, start: index, text });
      line += countNewlines(text);
      index = rawEnd;
      continue;
    }
    const quoteStart = quotedLiteralStart(source, index);
    if (quoteStart !== undefined) {
      const quote = source[quoteStart];
      let end = quoteStart + 1;
      while (end < source.length) {
        if (source[end] === "\n") line += 1;
        if (source[end] === "\\") {
          end += 2;
        } else if (source[end] === quote) {
          end += 1;
          break;
        } else {
          end += 1;
        }
      }
      if (end > source.length || source[end - 1] !== quote) {
        fail(`unterminated quoted literal at line ${line}`);
      }
      tokens.push({
        kind: "literal",
        line: line - countNewlines(source.slice(index, end)),
        start: index,
        text: source.slice(index, end),
      });
      index = end;
      continue;
    }
    if (isIdentifierStart(character)) {
      let end = index + 1;
      while (end < source.length && isIdentifierContinue(source[end])) end += 1;
      tokens.push({
        kind: "identifier",
        line,
        start: index,
        text: source.slice(index, end),
      });
      index = end;
      continue;
    }
    tokens.push({ kind: "punctuation", line, start: index, text: character });
    index += 1;
  }
  return tokens;
}

function rawStringEnd(source, index) {
  let cursor = index;
  if (source.startsWith("br", cursor) || source.startsWith("cr", cursor)) {
    cursor += 1;
  }
  if (source[cursor] !== "r") return undefined;
  cursor += 1;
  let hashes = 0;
  while (source[cursor] === "#") {
    hashes += 1;
    cursor += 1;
  }
  if (source[cursor] !== '"') return undefined;
  const terminator = `"${"#".repeat(hashes)}`;
  const end = source.indexOf(terminator, cursor + 1);
  if (end === -1) fail("unterminated raw string literal");
  return end + terminator.length;
}

function quotedLiteralStart(source, index) {
  if (source[index] === '"') return index;
  if (
    ["b", "c"].includes(source[index]) &&
    ["\"", "'"].includes(source[index + 1])
  ) {
    return index + 1;
  }
  if (source[index] !== "'") return undefined;
  // A Rust lifetime is tokenized as punctuation plus an identifier.
  if (/^[A-Za-z_][A-Za-z0-9_]*(?!')/.test(source.slice(index + 1))) {
    return undefined;
  }
  return index;
}

function isIdentifierStart(character) {
  return character !== undefined && /[A-Za-z_]/.test(character);
}

function isIdentifierContinue(character) {
  return character !== undefined && /[A-Za-z0-9_]/.test(character);
}

function fingerprint(tokens) {
  return createHash("sha256").update(JSON.stringify(tokens)).digest("hex");
}

function runCanaries() {
  const original = String.raw`fn old_name(value: &str) -> ! {
    // Formatting and comments are intentionally ignored.
    panic!("same {value}")
}`;
  const reformatted =
    'fn renamed(value:&str)->!{/* comment */panic!("same {value}")}';
  const changedLiteral =
    'fn renamed(value:&str)->!{/* comment */panic!("different {value}")}';
  const originalFingerprint = onlyFingerprint(original, "original canary");
  if (
    originalFingerprint !== onlyFingerprint(reformatted, "reformatted canary")
  ) {
    fail("helper fingerprint is not stable across renaming and formatting");
  }
  if (
    originalFingerprint === onlyFingerprint(changedLiteral, "literal canary")
  ) {
    fail("helper fingerprint did not preserve a changed string literal");
  }
}

function onlyFingerprint(source, label) {
  const helpers = extractFunctions(source, label);
  if (helpers.length !== 1) fail(`${label} did not contain exactly one helper`);
  return helpers[0].fingerprint;
}

function countNewlines(value) {
  return (value.match(/\n/g) ?? []).length;
}

function display(path) {
  const rootRelative = relative(root, path);
  if (!rootRelative.startsWith(`..${sep}`) && rootRelative !== "..") {
    return rootRelative.split(sep).join("/");
  }
  return path;
}

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}
