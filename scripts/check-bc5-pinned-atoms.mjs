#!/usr/bin/env node

import { createHash } from "node:crypto";
import { basename, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  GateError,
  decodeQuotedString,
  fail,
  findMatchingArrayEnd,
  findTopLevelConst,
  inspectCSource,
  inspectRustManifest,
  readRegularFile,
} from "./lib/bc5-gate-primitives.mjs";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const root = resolve(scriptDirectory, "..");
const manifestPath = resolve(
  root,
  "src/runtime/binary_object/pinned_atoms.rs",
);
const frozenManifestSha256 =
  "22fa629fc3204d3cb4d6cce5b73d3a3c813f46d5be32d64f92c348a9dfca1f20";
const allowedRustManifestAttributes = [
  "#[derive(::core::clone::Clone, ::core::marker::Copy, " +
    "::core::fmt::Debug, ::core::cmp::Eq, ::core::cmp::PartialEq,)]",
  "#[derive(::core::clone::Clone, ::core::marker::Copy, " +
    "::core::fmt::Debug, ::core::cmp::Eq, ::core::hash::Hash, " +
    "::core::cmp::Ord, ::core::cmp::PartialEq, ::core::cmp::PartialOrd,)]",
  "#[must_use]",
];
const allowedRustManifestUses = ["use super::wire::WireString;"];

try {
  main(process.argv.slice(2));
} catch (error) {
  if (error instanceof GateError) {
    console.error(`error: ${error.message}`);
    process.exit(error.status);
  }
  throw error;
}

function main(arguments_) {
  if (arguments_.length === 1 && arguments_[0] === "--self-test") {
    runSelfTests();
    const manifest = readRegularFile(manifestPath, "Rust pinned-atom manifest");
    const actual = parseRustManifest(manifest, manifestPath);
    assertFrozenManifest(actual.entries, manifestPath);
    console.log(
      `BC5 pinned atom parser self-tests and frozen manifest passed ` +
        `(${actual.entries.length} atoms, sha256=${frozenManifestSha256}).`,
    );
    return;
  }

  const sourcePath = parseArguments(arguments_);
  const source = readRegularFile(sourcePath, "QuickJS atom source");
  const manifest = readRegularFile(manifestPath, "Rust pinned-atom manifest");
  const expected = parseQuickJsAtoms(source, sourcePath);
  const actual = parseRustManifest(manifest, manifestPath);
  const failures = compareManifests(expected, actual);

  if (failures.length > 0) {
    fail(failures.map((failure) => `manifest mismatch: ${failure}`).join("\n"));
  }

  const expectedDigest = manifestSha256(expected);
  if (expectedDigest !== frozenManifestSha256) {
    fail(
      `authenticated QuickJS atom digest is ${expectedDigest}, ` +
        `expected frozen ${frozenManifestSha256}`,
    );
  }
  assertFrozenManifest(actual.entries, manifestPath);

  const stringEnd = expected.findLast((entry) => entry.kind === "String")?.id;
  const privateAtom = expected.find((entry) => entry.kind === "Private")?.id;
  const symbolStart = expected.find((entry) => entry.kind === "Symbol")?.id;
  console.log(
    `BC5 pinned atom manifest matches ${expected.length} QuickJS atoms ` +
      `(String 1..${stringEnd}, Private ${privateAtom}, ` +
      `Symbol ${symbolStart}..${expected.length}, ` +
      `sha256=${frozenManifestSha256}).`,
  );
}

function parseArguments(arguments_) {
  if (arguments_.length !== 2 || arguments_[0] !== "--source") {
    fail(`usage: ${basename(process.argv[1])} --source <quickjs-atom.h>`, 2);
  }
  return resolve(arguments_[1]);
}

function parseQuickJsAtoms(source, path) {
  const inspected = inspectCSource(source, path);
  const uncommentedLines = inspected.uncommented.split(/\r?\n/u);
  const structuralLines = inspected.structural.split(/\r?\n/u);
  if (uncommentedLines.length !== structuralLines.length) {
    fail(`${path}: internal C-source inspection line mismatch`);
  }

  const entries = [];
  const definition =
    /^\s*DEF\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*,\s*("(?:\\.|[^"\\])*")\s*\)\s*$/u;
  let state = "before";

  for (let index = 0; index < uncommentedLines.length; index += 1) {
    const line = uncommentedLines[index];
    const structural = structuralLines[index].trim();
    if (structural.length === 0 && line.trim().length === 0) {
      continue;
    }

    if (structural.startsWith("#")) {
      if (state === "before" && /^#\s*ifdef\s+DEF\s*$/u.test(structural)) {
        state = "entries";
        continue;
      }
      if (state === "entries" && /^#\s*endif\s*$/u.test(structural)) {
        state = "done";
        continue;
      }
      fail(
        `${path}:${index + 1}: unexpected preprocessor directive in ${state} section`,
      );
    }

    const match = definition.exec(line);
    if (match !== null) {
      if (state !== "entries") {
        fail(`${path}:${index + 1}: QuickJS DEF entry is outside the DEF section`);
      }
      const name = match[1];
      const kind =
        name === "Private_brand"
          ? "Private"
          : name.startsWith("Symbol_")
            ? "Symbol"
            : "String";
      entries.push({
        id: entries.length + 1,
        kind,
        line: index + 1,
        name,
        spelling: decodeQuotedString(match[2], `${path}:${index + 1}`),
      });
      continue;
    }

    if (/\bDEF\s*\(/u.test(line)) {
      fail(`${path}:${index + 1}: malformed QuickJS DEF entry`);
    }
    fail(`${path}:${index + 1}: unexpected content in ${state} section`);
  }

  if (state !== "done") {
    fail(`${path}: incomplete QuickJS DEF section (ended in ${state})`);
  }

  if (entries.length === 0) {
    fail(`${path}: no QuickJS DEF entries found`);
  }
  validateQuickJsKinds(entries, path);
  return entries;
}

function validateQuickJsKinds(entries, path) {
  const privateEntries = entries.filter((entry) => entry.kind === "Private");
  if (privateEntries.length !== 1) {
    fail(`${path}: expected exactly one Private_brand entry`);
  }
  const privateIndex = privateEntries[0].id - 1;
  const stringTail = entries
    .slice(0, privateIndex)
    .find((entry) => entry.kind !== "String");
  if (stringTail !== undefined) {
    fail(`${path}:${stringTail.line}: non-string atom precedes Private_brand`);
  }
  const symbolTail = entries
    .slice(privateIndex + 1)
    .find((entry) => entry.kind !== "Symbol");
  if (symbolTail !== undefined) {
    fail(`${path}:${symbolTail.line}: non-symbol atom follows Private_brand`);
  }
  if (privateIndex === entries.length - 1) {
    fail(`${path}: pinned atom source has no Symbol entries`);
  }
}

function parseRustManifest(source, path) {
  const manifest = inspectRustManifest(
    source,
    path,
    "atom",
    allowedRustManifestAttributes,
    allowedRustManifestUses,
  );
  const { uncommented, structural } = manifest;

  const count = parseRustU32Constant(manifest, "PINNED_ATOM_COUNT");
  const lastString = parseRustU32Constant(manifest, "LAST_STRING_ATOM");
  const privateAtom = parseRustU32Constant(manifest, "PRIVATE_ATOM");
  const dynamicIndex = findTopLevelConst(manifest, "FIRST_DYNAMIC_ATOM");
  const dynamicExpression =
    /^const\s+FIRST_DYNAMIC_ATOM\s*:\s*u32\s*=\s*PINNED_ATOM_COUNT\s*\+\s*1\s*;/u;
  if (!dynamicExpression.test(structural.slice(dynamicIndex))) {
    fail(`${path}: FIRST_DYNAMIC_ATOM must equal PINNED_ATOM_COUNT + 1`);
  }

  const arrayIndex = findTopLevelConst(manifest, "PINNED_ATOM_SPELLINGS");
  const arrayStart =
    /^const\s+PINNED_ATOM_SPELLINGS\s*:\s*\[\s*&str\s*;\s*PINNED_ATOM_COUNT\s+as\s+usize\s*\]\s*=\s*\[/u;
  const startMatch = arrayStart.exec(structural.slice(arrayIndex));
  if (startMatch === null) {
    fail(`${path}: PINNED_ATOM_SPELLINGS must be a direct fixed array`);
  }
  const open = arrayIndex + startMatch[0].lastIndexOf("[");
  const end = findMatchingArrayEnd(manifest, open, "PINNED_ATOM_SPELLINGS");
  const afterArray = structural.slice(end + 1).match(/^\s*/u)?.[0].length ?? 0;
  if (structural[end + 1 + afterArray] !== ";") {
    fail(`${path}: PINNED_ATOM_SPELLINGS array must end with a semicolon`);
  }
  const spellings = parseRustStringArray(
    uncommented.slice(open + 1, end),
    path,
  );

  const entries = spellings.map((spelling, index) => {
    const id = index + 1;
    const kind =
      id <= lastString
        ? "String"
        : id === privateAtom
          ? "Private"
          : "Symbol";
    return { id, kind, spelling };
  });
  return { count, entries, lastString, privateAtom };
}

function parseRustU32Constant(manifest, name) {
  const { path, structural } = manifest;
  const index = findTopLevelConst(manifest, name);
  const expression = new RegExp(
    `^const\\s+${name}\\s*:\\s*u32\\s*=\\s*([0-9][0-9_]*)\\s*;`,
    "u",
  );
  const match = expression.exec(structural.slice(index));
  if (match === null) {
    fail(`${path}: ${name} must be one direct numeric u32 constant`);
  }
  const value = Number.parseInt(match[1].replaceAll("_", ""), 10);
  if (!Number.isSafeInteger(value)) {
    fail(`${path}: ${name} is outside JavaScript's safe integer range`);
  }
  return value;
}

function parseRustStringArray(body, path) {
  const spellings = [];
  let offset = 0;
  while (offset < body.length) {
    const whitespace = /^(?:\s|\/\/[^\n]*(?:\n|$)|\/\*[\s\S]*?\*\/|,)+/u.exec(
      body.slice(offset),
    );
    if (whitespace !== null) {
      offset += whitespace[0].length;
      continue;
    }
    if (offset === body.length) {
      break;
    }
    const literal = /^"(?:\\.|[^"\\])*"/u.exec(body.slice(offset));
    if (literal === null) {
      fail(`${path}: unsupported token in PINNED_ATOM_SPELLINGS at byte ${offset}`);
    }
    spellings.push(decodeQuotedString(literal[0], `${path} manifest entry`));
    offset += literal[0].length;
  }
  return spellings;
}

function compareManifests(expected, actual) {
  const failures = [];
  if (actual.count !== expected.length) {
    failures.push(
      `PINNED_ATOM_COUNT is ${actual.count}, QuickJS defines ${expected.length}`,
    );
  }
  if (actual.entries.length !== actual.count) {
    failures.push(
      `PINNED_ATOM_SPELLINGS has ${actual.entries.length} entries, ` +
        `PINNED_ATOM_COUNT is ${actual.count}`,
    );
  }

  const expectedLastString = expected.findLast(
    (entry) => entry.kind === "String",
  )?.id;
  const expectedPrivate = expected.find(
    (entry) => entry.kind === "Private",
  )?.id;
  if (actual.lastString !== expectedLastString) {
    failures.push(
      `LAST_STRING_ATOM is ${actual.lastString}, expected ${expectedLastString}`,
    );
  }
  if (actual.privateAtom !== expectedPrivate) {
    failures.push(
      `PRIVATE_ATOM is ${actual.privateAtom}, expected ${expectedPrivate}`,
    );
  }

  const comparedLength = Math.min(expected.length, actual.entries.length);
  for (let index = 0; index < comparedLength; index += 1) {
    const wanted = expected[index];
    const found = actual.entries[index];
    if (found.id !== wanted.id) {
      failures.push(`manifest entry ${index + 1} has ID ${found.id}`);
    }
    if (found.kind !== wanted.kind) {
      failures.push(
        `atom ${wanted.id} ${wanted.name} kind is ${found.kind}, ` +
          `expected ${wanted.kind}`,
      );
    }
    if (found.spelling !== wanted.spelling) {
      failures.push(
        `atom ${wanted.id} ${wanted.name} spelling is ` +
          `${JSON.stringify(found.spelling)}, expected ` +
          JSON.stringify(wanted.spelling),
      );
    }
    if (failures.length >= 20) {
      failures.push("additional manifest differences omitted");
      break;
    }
  }
  return failures;
}

function manifestSha256(entries) {
  const canonical = entries
    .map(
      (entry) =>
        `${entry.id}\t${entry.kind}\t${JSON.stringify(entry.spelling)}\n`,
    )
    .join("");
  return createHash("sha256").update(canonical).digest("hex");
}

function assertFrozenManifest(entries, path) {
  const digest = manifestSha256(entries);
  if (digest !== frozenManifestSha256) {
    fail(
      `${path}: active manifest sha256 is ${digest}, ` +
        `expected frozen ${frozenManifestSha256}`,
    );
  }
}

function runSelfTests() {
  runQuickJsParserSelfTests();
  runRustParserSelfTests();
}

function selfTestAtoms() {
  return [
    { id: 1, kind: "String", name: "alpha", spelling: "alpha" },
    { id: 2, kind: "Private", name: "Private_brand", spelling: "private" },
    { id: 3, kind: "Symbol", name: "Symbol_probe", spelling: "symbol" },
  ];
}

function runQuickJsParserSelfTests() {
  const expected = selfTestAtoms();
  const valid = selfTestQuickJsSource(["alpha", "private", "symbol"]);
  const wrong = selfTestQuickJsSource(["wrong", "private", "symbol"]);
  const parsed = parseQuickJsAtoms(valid, "self-test valid QuickJS source");
  if (manifestSha256(parsed) !== manifestSha256(expected)) {
    fail("self-test valid QuickJS atom source did not round-trip");
  }

  const commentDecoy = parseQuickJsAtoms(
    `/* ${valid.replace("#endif /* DEF */", "#endif")} */\n${wrong}`,
    "self-test QuickJS comment decoy",
  );
  if (manifestSha256(commentDecoy) === manifestSha256(expected)) {
    fail("self-test QuickJS comment decoy hid the active wrong source");
  }

  const malformedCases = [
    ["C line continuation", `// hidden \\\n${valid}`],
    ["C bare-CR continuation", `// hidden \\\r${valid}`],
    ["C trigraph", `// hidden ??/\n${valid}`],
    ["inactive DEF section", valid.replace("#ifdef DEF", "#if 0")],
    [
      "wrong DEF guard",
      valid.replace("#ifdef DEF", "#ifdef NEVER_DEFINED"),
    ],
    [
      "early DEF undef",
      valid.replace(
        'DEF(Private_brand, "private")',
        '#undef DEF\nDEF(Private_brand, "private")',
      ),
    ],
    [
      "empty DEF definition",
      valid.replace(
        'DEF(Private_brand, "private")',
        '#define DEF(name, value)\nDEF(Private_brand, "private")',
      ),
    ],
    ["alternate DEF branch", valid.replace("#endif /* DEF */", "#else\n#endif")],
    ["missing DEF tail", valid.replace("#endif /* DEF */\n", "")],
    ["entry before DEF section", `DEF(decoy, "decoy")\n${valid}`],
    ["entry after DEF section", `${valid}DEF(decoy, "decoy")\n`],
    ["include before DEF section", `#include "decoy.h"\n${valid}`],
    ["active token before DEF section", `int decoy;\n${valid}`],
    ["string decoy before DEF section", `"${escapeForCString(valid)}"\n${wrong}`],
    [
      "malformed DEF entry",
      valid.replace('DEF(alpha, "alpha")', "DEF(alpha, nope)"),
    ],
  ];
  for (const [label, source] of malformedCases) {
    expectGateFailure(`QuickJS ${label}`, () =>
      parseQuickJsAtoms(source, `self-test QuickJS ${label}`),
    );
  }
}

function selfTestQuickJsSource(spellings) {
  return `
// self-test atom header
#ifdef DEF
DEF(alpha, ${JSON.stringify(spellings[0])})
DEF(Private_brand, ${JSON.stringify(spellings[1])})
DEF(Symbol_probe, ${JSON.stringify(spellings[2])})
#endif /* DEF */
`;
}

function escapeForCString(value) {
  return value
    .replaceAll("\\", "\\\\")
    .replaceAll('"', '\\"')
    .replaceAll("\n", "\\n");
}

function runRustParserSelfTests() {
  const expected = selfTestAtoms();
  const valid = selfTestManifest(["alpha", "private", "symbol"]);
  const wrong = selfTestManifest(["wrong", "private", "symbol"]);
  const parsed = parseRustManifest(valid, "self-test valid manifest");
  if (compareManifests(expected, parsed).length !== 0) {
    fail("self-test valid manifest did not round-trip");
  }

  const withTests = parseRustManifest(
    `${valid}\n#[cfg(test)]\nmod tests { const DECOY: &str = "ignored"; }\n`,
    "self-test valid tests tail",
  );
  if (compareManifests(expected, withTests).length !== 0) {
    fail("self-test valid top-level tests tail changed the active manifest");
  }

  const multilineVisibility = parseRustManifest(
    valid.replace(
      "const PINNED_ATOM_COUNT",
      "pub(crate)\nconst PINNED_ATOM_COUNT",
    ),
    "self-test multiline visibility",
  );
  if (compareManifests(expected, multilineVisibility).length !== 0) {
    fail("self-test multiline visibility changed the active manifest");
  }

  const commentDecoy = parseRustManifest(
    `/* ${valid} */\n${wrong}`,
    "self-test comment decoy",
  );
  if (compareManifests(expected, commentDecoy).length === 0) {
    fail("self-test comment decoy hid the active wrong manifest");
  }

  for (const [label, prefix] of [
    ["raw string decoy", `const DECOY: &str = r###"${valid}"###;\n`],
    ["raw byte string decoy", `const DECOY: &[u8] = br###"${valid}"###;\n`],
    [
      "ordinary byte string decoy",
      'const DECOY: &[u8] = b"const PINNED_ATOM_COUNT: u32 = 3;";\n',
    ],
    [
      "character literal decoy",
      "const LEFT: char = '['; const RIGHT: u8 = b'}';\n",
    ],
    [
      "tests marker string decoy",
      `const DECOY: &str = r###"#[cfg(test)] mod tests {} ${valid}"###;\n`,
    ],
  ]) {
    const active = parseRustManifest(
      `${prefix}${wrong}`,
      `self-test ${label}`,
    );
    if (compareManifests(expected, active).length === 0) {
      fail(`self-test ${label} hid the active wrong manifest`);
    }
  }

  expectGateFailure("conditional decoy", () =>
    parseRustManifest(
      `#[cfg(any())]\n${valid}\n${wrong}`,
      "self-test conditional decoy",
    ),
  );
  expectGateFailure("duplicate decoy", () =>
    parseRustManifest(`${valid}\n${wrong}`, "self-test duplicate decoy"),
  );
  expectGateFailure("nested decoy", () =>
    parseRustManifest(
      `mod decoy {\n${valid}\n}\n${wrong}`,
      "self-test nested decoy",
    ),
  );
  expectGateFailure("inner attribute decoy", () =>
    parseRustManifest(
      `#![cfg(any())]\n${valid}`,
      "self-test inner attribute decoy",
    ),
  );
  expectGateFailure("attribute before multiline visibility", () =>
    parseRustManifest(
      valid.replace(
        "const PINNED_ATOM_COUNT",
        "#[evil]\npub(crate)\nconst PINNED_ATOM_COUNT",
      ),
      "self-test attribute before multiline visibility",
    ),
  );
  expectGateFailure("unrelated production attribute", () =>
    parseRustManifest(
      `#[evil]\nstruct Decoy;\n${valid}`,
      "self-test unrelated production attribute",
    ),
  );
  expectGateFailure("derive-shadowing import", () =>
    parseRustManifest(
      `use evil::Clone;\n${valid}`,
      "self-test derive-shadowing import",
    ),
  );
  expectGateFailure("extern crate macro import", () =>
    parseRustManifest(
      `extern crate evil;\n${valid}`,
      "self-test extern crate macro import",
    ),
  );
  expectGateFailure("qualified function-like macro", () =>
    parseRustManifest(
      `evil::replace_manifest!();\n${valid}`,
      "self-test qualified function-like macro",
    ),
  );
  expectGateFailure("Unicode function-like macro", () =>
    parseRustManifest(
      `evil::\u6076\u610f!();\n${valid}`,
      "self-test Unicode function-like macro",
    ),
  );
  expectGateFailure("production unary exclamation", () =>
    parseRustManifest(
      `const DECOY: bool = !(false);\n${valid}`,
      "self-test production unary exclamation",
    ),
  );
  expectGateFailure("parenthesized macro wrapper", () =>
    parseRustManifest(
      `passthrough!(\n${valid}\n);`,
      "self-test parenthesized macro wrapper",
    ),
  );
  expectGateFailure("bracket macro wrapper", () =>
    parseRustManifest(
      `passthrough![\n${valid}\n];`,
      "self-test bracket macro wrapper",
    ),
  );
  expectGateFailure("include decoy", () =>
    parseRustManifest(
      `const DECOY: &[u8] = include_bytes!("decoy");\n${wrong}`,
      "self-test include decoy",
    ),
  );
  expectGateFailure("macro definition decoy", () =>
    parseRustManifest(
      `macro_rules! decoy { () => { ${valid} } }\n${wrong}`,
      "self-test macro definition decoy",
    ),
  );
  expectGateFailure("tests cutoff macro bypass", () =>
    parseRustManifest(
      `discard!(\n${valid}\n);\n` +
        `cutoff!(#[cfg(test)] mod tests {});\n${wrong}`,
      "self-test tests cutoff macro bypass",
    ),
  );
  expectGateFailure("production after tests tail", () =>
    parseRustManifest(
      `${valid}\n#[cfg(test)]\nmod tests {}\nconst TAIL: u8 = 1;\n`,
      "self-test production after tests tail",
    ),
  );

  for (const name of [
    "PINNED_ATOM_COUNT",
    "FIRST_DYNAMIC_ATOM",
    "LAST_STRING_ATOM",
    "PRIVATE_ATOM",
    "PINNED_ATOM_SPELLINGS",
  ]) {
    expectGateFailure(`${name} attribute decoy`, () =>
      parseRustManifest(
        valid.replace(`const ${name}`, `#[evil]\nconst ${name}`),
        `self-test ${name} attribute decoy`,
      ),
    );
  }

  const wrongActive = parseRustManifest(wrong, "self-test wrong active");
  if (compareManifests(expected, wrongActive).length === 0) {
    fail("self-test wrong active manifest escaped semantic comparison");
  }
}

function selfTestManifest(spellings) {
  return `
use super::wire::WireString;

const PINNED_ATOM_COUNT: u32 = 3;
const FIRST_DYNAMIC_ATOM: u32 = PINNED_ATOM_COUNT + 1;
const LAST_STRING_ATOM: u32 = 1;
const PRIVATE_ATOM: u32 = 2;
const PINNED_ATOM_SPELLINGS: [&str; PINNED_ATOM_COUNT as usize] = [
    ${spellings.map((spelling) => JSON.stringify(spelling)).join(",\n    ")},
];
`;
}

function expectGateFailure(label, operation) {
  try {
    operation();
  } catch (error) {
    if (error instanceof GateError) {
      return;
    }
    throw error;
  }
  fail(`self-test ${label} unexpectedly passed`);
}
