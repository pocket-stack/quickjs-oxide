#!/usr/bin/env node

import { createHash } from "node:crypto";
import { lstatSync, readFileSync } from "node:fs";
import { basename, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const root = resolve(scriptDirectory, "..");
const manifestPath = resolve(
  root,
  "src/runtime/binary_object/pinned_atoms.rs",
);
const frozenManifestSha256 =
  "22fa629fc3204d3cb4d6cce5b73d3a3c813f46d5be32d64f92c348a9dfca1f20";

class GateError extends Error {
  constructor(message, status = 1) {
    super(message);
    this.name = "GateError";
    this.status = status;
  }
}

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

function readRegularFile(path, label) {
  let metadata;
  try {
    metadata = lstatSync(path);
  } catch (error) {
    fail(`${label} is unavailable at ${path}: ${error.message}`);
  }
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    fail(`${label} must be a regular non-symlink file: ${path}`);
  }
  return readFileSync(path, "utf8");
}

function parseQuickJsAtoms(source, path) {
  const entries = [];
  const lines = source.split(/\r?\n/u);
  const definition =
    /^\s*DEF\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*,\s*("(?:\\.|[^"\\])*")\s*\)\s*(?:\/\*.*\*\/)?\s*$/u;

  for (const [index, line] of lines.entries()) {
    if (!line.includes("DEF(")) {
      continue;
    }
    const match = definition.exec(line);
    if (match === null) {
      fail(`${path}:${index + 1}: malformed QuickJS DEF entry`);
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

function lexRust(source, path) {
  const uncommented = source.split("");
  const structural = source.split("");
  let index = 0;

  while (index < source.length) {
    if (source.startsWith("//", index)) {
      const end = source.indexOf("\n", index + 2);
      const stop = end === -1 ? source.length : end;
      blankRange(uncommented, index, stop);
      blankRange(structural, index, stop);
      index = stop;
      continue;
    }
    if (source.startsWith("/*", index)) {
      let depth = 1;
      let cursor = index + 2;
      while (cursor < source.length && depth !== 0) {
        if (source.startsWith("/*", cursor)) {
          depth += 1;
          cursor += 2;
        } else if (source.startsWith("*/", cursor)) {
          depth -= 1;
          cursor += 2;
        } else {
          cursor += 1;
        }
      }
      if (depth !== 0) {
        fail(`${path}: unterminated Rust block comment`);
      }
      blankRange(uncommented, index, cursor);
      blankRange(structural, index, cursor);
      index = cursor;
      continue;
    }

    const raw = /^(?:br|r)(#{0,255})"/u.exec(source.slice(index));
    if (raw !== null) {
      const terminator = `"${raw[1]}`;
      const contentStart = index + raw[0].length;
      const closing = source.indexOf(terminator, contentStart);
      if (closing === -1) {
        fail(`${path}: unterminated Rust raw string`);
      }
      const stop = closing + terminator.length;
      blankRange(structural, index, stop);
      index = stop;
      continue;
    }

    if (source[index] === '"') {
      let cursor = index + 1;
      let escaped = false;
      while (cursor < source.length) {
        const character = source[cursor];
        if (!escaped && character === '"') {
          cursor += 1;
          break;
        }
        if (!escaped && (character === "\n" || character === "\r")) {
          fail(`${path}: newline in ordinary Rust string`);
        }
        escaped = !escaped && character === "\\";
        if (character !== "\\") {
          escaped = false;
        }
        cursor += 1;
      }
      if (cursor > source.length || source[cursor - 1] !== '"') {
        fail(`${path}: unterminated ordinary Rust string`);
      }
      blankRange(structural, index, cursor);
      index = cursor;
      continue;
    }

    const characterLiteral = /^'(?:\\.|[^'\\\r\n])'/u.exec(
      source.slice(index),
    );
    if (characterLiteral !== null) {
      const stop = index + characterLiteral[0].length;
      blankRange(structural, index, stop);
      index = stop;
      continue;
    }

    index += 1;
  }

  return {
    uncommented: uncommented.join(""),
    structural: structural.join(""),
  };
}

function blankRange(characters, start, end) {
  for (let index = start; index < end; index += 1) {
    if (characters[index] !== "\n" && characters[index] !== "\r") {
      characters[index] = " ";
    }
  }
}

function parseRustManifest(source, path) {
  const lexed = lexRust(source, path);
  const testModule = /#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*mod\s+tests\b/u.exec(
    lexed.structural,
  );
  const productionEnd = testModule?.index ?? source.length;
  const uncommented = lexed.uncommented.slice(0, productionEnd);
  const structural = lexed.structural.slice(0, productionEnd);
  rejectManifestIndirection(structural, path);
  const braceDepth = computeBraceDepth(structural, path);

  const count = parseRustU32Constant(
    structural,
    braceDepth,
    path,
    "PINNED_ATOM_COUNT",
  );
  const lastString = parseRustU32Constant(
    structural,
    braceDepth,
    path,
    "LAST_STRING_ATOM",
  );
  const privateAtom = parseRustU32Constant(
    structural,
    braceDepth,
    path,
    "PRIVATE_ATOM",
  );
  const dynamicIndex = uniqueTopLevelConst(
    structural,
    braceDepth,
    path,
    "FIRST_DYNAMIC_ATOM",
  );
  const dynamicExpression =
    /^const\s+FIRST_DYNAMIC_ATOM\s*:\s*u32\s*=\s*PINNED_ATOM_COUNT\s*\+\s*1\s*;/u;
  if (!dynamicExpression.test(structural.slice(dynamicIndex))) {
    fail(`${path}: FIRST_DYNAMIC_ATOM must equal PINNED_ATOM_COUNT + 1`);
  }

  const arrayIndex = uniqueTopLevelConst(
    structural,
    braceDepth,
    path,
    "PINNED_ATOM_SPELLINGS",
  );
  const arrayStart =
    /^const\s+PINNED_ATOM_SPELLINGS\s*:\s*\[\s*&str\s*;\s*PINNED_ATOM_COUNT\s+as\s+usize\s*\]\s*=\s*\[/u;
  const startMatch = arrayStart.exec(structural.slice(arrayIndex));
  if (startMatch === null) {
    fail(`${path}: PINNED_ATOM_SPELLINGS must be a direct fixed array`);
  }
  const open = arrayIndex + startMatch[0].lastIndexOf("[");
  const end = findMatchingArrayEnd(structural, open, path);
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

function parseRustU32Constant(structural, braceDepth, path, name) {
  const index = uniqueTopLevelConst(structural, braceDepth, path, name);
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

function uniqueTopLevelConst(structural, braceDepth, path, name) {
  const expression = new RegExp(`\\bconst\\s+${name}\\b`, "gu");
  const matches = [...structural.matchAll(expression)];
  if (matches.length !== 1) {
    fail(`${path}: expected exactly one ${name} const, found ${matches.length}`);
  }
  const index = matches[0].index;
  if (braceDepth[index] !== 0) {
    fail(`${path}: ${name} must be defined at module top level`);
  }
  return index;
}

function rejectManifestIndirection(structural, path) {
  const forbidden = [
    [/#\s*\[\s*cfg(?:_attr)?\b/u, "conditional compilation"],
    [/\bcfg\s*!/u, "cfg!"],
    [/\b(?:include|include_str|include_bytes)\s*!/u, "include macro"],
    [/\bmacro_rules\s*!/u, "macro definition"],
  ];
  for (const [pattern, label] of forbidden) {
    if (pattern.test(structural)) {
      fail(`${path}: ${label} is not allowed in the production atom manifest`);
    }
  }
}

function computeBraceDepth(structural, path) {
  const depthAt = new Uint32Array(structural.length + 1);
  let depth = 0;
  for (let index = 0; index < structural.length; index += 1) {
    depthAt[index] = depth;
    if (structural[index] === "{") {
      depth += 1;
    } else if (structural[index] === "}") {
      if (depth === 0) {
        fail(`${path}: unmatched closing brace in atom manifest`);
      }
      depth -= 1;
    }
  }
  depthAt[structural.length] = depth;
  if (depth !== 0) {
    fail(`${path}: unmatched opening brace in atom manifest`);
  }
  return depthAt;
}

function findMatchingArrayEnd(structural, open, path) {
  let depth = 0;
  for (let index = open; index < structural.length; index += 1) {
    if (structural[index] === "[") {
      depth += 1;
    } else if (structural[index] === "]") {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }
  fail(`${path}: unterminated PINNED_ATOM_SPELLINGS array`);
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

function decodeQuotedString(literal, location) {
  try {
    return JSON.parse(literal);
  } catch (error) {
    fail(`${location}: unsupported string literal ${literal}: ${error.message}`);
  }
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
  const expected = [
    { id: 1, kind: "String", name: "alpha", spelling: "alpha" },
    { id: 2, kind: "Private", name: "Private_brand", spelling: "private" },
    { id: 3, kind: "Symbol", name: "Symbol_probe", spelling: "symbol" },
  ];
  const valid = selfTestManifest(["alpha", "private", "symbol"]);
  const wrong = selfTestManifest(["wrong", "private", "symbol"]);
  const parsed = parseRustManifest(valid, "self-test valid manifest");
  if (compareManifests(expected, parsed).length !== 0) {
    fail("self-test valid manifest did not round-trip");
  }

  const commentDecoy = parseRustManifest(
    `/* ${valid} */\n${wrong}`,
    "self-test comment decoy",
  );
  if (compareManifests(expected, commentDecoy).length === 0) {
    fail("self-test comment decoy hid the active wrong manifest");
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

  const wrongActive = parseRustManifest(wrong, "self-test wrong active");
  if (compareManifests(expected, wrongActive).length === 0) {
    fail("self-test wrong active manifest escaped semantic comparison");
  }
}

function selfTestManifest(spellings) {
  return `
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

function fail(message, status = 1) {
  throw new GateError(message, status);
}
