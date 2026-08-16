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
  "src/runtime/binary_object/pinned_opcodes.rs",
);
const frozenManifestSha256 =
  "e2fba2aea6f6e898d21a164d9917858c7aa95537f9d36d47aadb93399614af14";
const allowedRustManifestAttributes = [
  "#[derive(::core::clone::Clone, ::core::marker::Copy, " +
    "::core::fmt::Debug, ::core::cmp::Eq, ::core::hash::Hash, " +
    "::core::cmp::PartialEq,)]",
  "#[derive(::core::clone::Clone, ::core::marker::Copy, " +
    "::core::fmt::Debug, ::core::cmp::Eq, ::core::hash::Hash, " +
    "::core::cmp::Ord, ::core::cmp::PartialEq, ::core::cmp::PartialOrd,)]",
  "#[derive(::core::clone::Clone, ::core::marker::Copy, " +
    "::core::fmt::Debug, ::core::cmp::Eq, ::core::cmp::PartialEq,)]",
  "#[must_use]",
  "#[rustfmt::skip]",
];

const quickJsFormats = [
  ["none", "None"],
  ["none_int", "NoneInt"],
  ["none_loc", "NoneLoc"],
  ["none_arg", "NoneArg"],
  ["none_var_ref", "NoneVarRef"],
  ["u8", "U8"],
  ["i8", "I8"],
  ["loc8", "Loc8"],
  ["const8", "Const8"],
  ["label8", "Label8"],
  ["u16", "U16"],
  ["i16", "I16"],
  ["label16", "Label16"],
  ["npop", "NPop"],
  ["npopx", "NPopX"],
  ["npop_u16", "NPopU16"],
  ["loc", "Loc"],
  ["arg", "Arg"],
  ["var_ref", "VarRef"],
  ["u32", "U32"],
  ["i32", "I32"],
  ["const", "Const"],
  ["label", "Label"],
  ["atom", "Atom"],
  ["atom_u8", "AtomU8"],
  ["atom_u16", "AtomU16"],
  ["atom_label_u8", "AtomLabelU8"],
  ["atom_label_u16", "AtomLabelU16"],
  ["label_u16", "LabelU16"],
];
const quickJsFormatNames = quickJsFormats.map(([name]) => name);
const rustFormatNames = new Map(
  quickJsFormats.map(([quickJs, rust]) => [rust, quickJs]),
);
const atomBearingOpcodeIds = [
  4, 5, 49, 61, 62, 63, 73, 74, 81, 83, 84, 113, 114, 115, 116, 117,
  118, 119, 120, 121, 151,
];
const expectedTemporaryDescriptors = [
  ["enter_scope", 3, 0, 0, "u16"],
  ["leave_scope", 3, 0, 0, "u16"],
  ["label", 5, 0, 0, "label"],
  ["scope_get_var_undef", 7, 0, 1, "atom_u16"],
  ["scope_get_var", 7, 0, 1, "atom_u16"],
  ["scope_put_var", 7, 1, 0, "atom_u16"],
  ["scope_delete_var", 7, 0, 1, "atom_u16"],
  ["scope_make_ref", 11, 0, 2, "atom_label_u16"],
  ["scope_get_ref", 7, 0, 2, "atom_u16"],
  ["scope_put_var_init", 7, 0, 2, "atom_u16"],
  ["scope_get_var_checkthis", 7, 0, 1, "atom_u16"],
  ["scope_get_private_field", 7, 1, 1, "atom_u16"],
  ["scope_get_private_field2", 7, 1, 2, "atom_u16"],
  ["scope_put_private_field", 7, 2, 0, "atom_u16"],
  ["scope_in_private_field", 7, 1, 1, "atom_u16"],
  ["get_field_opt_chain", 5, 1, 1, "atom"],
  ["get_array_el_opt_chain", 1, 2, 1, "none"],
  ["set_class_name", 5, 1, 1, "u32"],
  ["line_num", 5, 0, 0, "u32"],
].map(([name, size, nPop, nPush, format], index) => ({
  format,
  id: index,
  nPop,
  nPush,
  name,
  size,
}));

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
    const manifest = readRegularFile(manifestPath, "Rust pinned-opcode manifest");
    const actual = parseRustManifest(manifest, manifestPath);
    validateWireCatalog(actual.entries, manifestPath);
    assertFrozenManifest(actual.entries, manifestPath);
    console.log(
      `BC5 pinned opcode parser self-tests and frozen manifest passed ` +
        `(${actual.entries.length} opcodes, ` +
        `${atomBearingOpcodeIds.length} atom-bearing, ` +
        `sha256=${frozenManifestSha256}).`,
    );
    return;
  }

  const sourcePath = parseArguments(arguments_);
  const source = readRegularFile(sourcePath, "QuickJS opcode source");
  const manifest = readRegularFile(manifestPath, "Rust pinned-opcode manifest");
  const expected = parseQuickJsOpcodes(source, sourcePath);
  const actual = parseRustManifest(manifest, manifestPath);
  validateWireCatalog(actual.entries, manifestPath);
  const failures = compareCatalogs(expected.entries, actual);

  if (failures.length > 0) {
    fail(failures.map((failure) => `manifest mismatch: ${failure}`).join("\n"));
  }

  const expectedDigest = manifestSha256(expected.entries);
  if (expectedDigest !== frozenManifestSha256) {
    fail(
      `authenticated QuickJS opcode digest is ${expectedDigest}, ` +
        `expected frozen ${frozenManifestSha256}`,
    );
  }
  assertFrozenManifest(actual.entries, manifestPath);

  console.log(
    `BC5 pinned opcode manifest matches ${expected.entries.length} ` +
      `QuickJS wire opcodes (${expected.formats.length} formats, ` +
      `${expected.temporaryEntries.length} temporary descriptors skipped, ` +
      `${atomBearingOpcodeIds.length} atom-bearing at operand +1, ` +
      `sha256=${frozenManifestSha256}).`,
  );
}

function parseArguments(arguments_) {
  if (arguments_.length !== 2 || arguments_[0] !== "--source") {
    fail(`usage: ${basename(process.argv[1])} --source <quickjs-opcode.h>`, 2);
  }
  return resolve(arguments_[1]);
}

function parseQuickJsOpcodes(source, path) {
  const structural = inspectCSource(source, path).structural;
  const formats = [];
  const entries = [];
  const temporaryEntries = [];
  const descriptor =
    /^\s*(DEF|def)\(\s*([a-z_][a-z0-9_]*)\s*,\s*([0-9]+)\s*,\s*([0-9]+)\s*,\s*([0-9]+)\s*,\s*([a-z_][a-z0-9_]*)\s*\)\s*$/u;
  const format = /^\s*FMT\(\s*([a-z_][a-z0-9_]*)\s*\)\s*$/u;
  const lines = structural.split(/\r?\n/u);
  let state = "before_fmt";
  let index = 0;

  while (index < lines.length) {
    const line = lines[index];
    const trimmed = line.trim();
    if (trimmed.length === 0) {
      index += 1;
      continue;
    }
    if (trimmed.startsWith("#")) {
      const logical = readCLogicalDirective(lines, index, path);
      state = advanceQuickJsDirective(
        state,
        logical.text,
        path,
        index + 1,
        formats,
        entries,
        temporaryEntries,
      );
      index = logical.endIndex + 1;
      continue;
    }
    if (/\\\s*$/u.test(line)) {
      fail(`${path}:${index + 1}: continuation outside a preprocessor directive`);
    }

    const formatMatch = format.exec(line);
    if (formatMatch !== null) {
      if (state !== "formats") {
        fail(`${path}:${index + 1}: FMT entry is outside the #ifdef FMT section`);
      }
      formats.push({ line: index + 1, name: formatMatch[1] });
      index += 1;
      continue;
    }

    const descriptorMatch = descriptor.exec(line);
    if (descriptorMatch !== null) {
      const macro = descriptorMatch[1];
      let target;
      if (state === "final_long" && macro === "DEF") {
        target = entries;
      } else if (state === "final_long" && macro === "def") {
        if (entries.length !== 178 || entries[177]?.name !== "nop") {
          fail(
            `${path}:${index + 1}: temporary descriptors must follow final nop 177`,
          );
        }
        state = "temporaries";
        target = temporaryEntries;
      } else if (state === "temporaries" && macro === "def") {
        target = temporaryEntries;
      } else if (state === "short" && macro === "DEF") {
        target = entries;
      } else {
        fail(
          `${path}:${index + 1}: ${macro} descriptor is not active in ${state} section`,
        );
      }
      target.push({
        format: descriptorMatch[6],
        id: target.length,
        line: index + 1,
        nPop: parseCInteger(descriptorMatch[4], path, index + 1),
        nPush: parseCInteger(descriptorMatch[5], path, index + 1),
        name: descriptorMatch[2],
        size: parseCInteger(descriptorMatch[3], path, index + 1),
      });
      index += 1;
      continue;
    }

    if (/\b(?:FMT|DEF|def)\s*\(/u.test(line)) {
      fail(`${path}:${index + 1}: malformed QuickJS opcode entry`);
    }
    fail(`${path}:${index + 1}: unexpected content in pinned opcode header`);
  }

  if (state !== "done") {
    fail(`${path}: incomplete pinned opcode preprocessor structure (ended in ${state})`);
  }
  validateQuickJsSource(formats, entries, temporaryEntries, path);
  return { entries, formats, temporaryEntries };
}

function readCLogicalDirective(lines, startIndex, path) {
  const parts = [];
  let index = startIndex;
  while (index < lines.length) {
    const continued = /\\\s*$/u.test(lines[index]);
    parts.push(lines[index].replace(/\\\s*$/u, ""));
    if (!continued) {
      return { endIndex: index, text: parts.join(" ").trim() };
    }
    index += 1;
  }
  fail(`${path}:${startIndex + 1}: unterminated preprocessor continuation`);
}

function advanceQuickJsDirective(
  state,
  text,
  path,
  line,
  formats,
  entries,
  temporaryEntries,
) {
  const parsed = /^#\s*([A-Za-z_][A-Za-z0-9_]*)([\s\S]*)$/u.exec(text);
  if (parsed === null) {
    fail(`${path}:${line}: malformed preprocessor directive`);
  }
  const directive = parsed[1];
  const arguments_ = parsed[2].trim();
  const matches = (name, expression) =>
    directive === name && expression.test(arguments_);

  switch (state) {
    case "before_fmt":
      if (matches("ifdef", /^FMT$/u)) return "formats";
      break;
    case "formats":
      if (matches("undef", /^FMT$/u) && formats.length === 29) {
        return "after_fmt_undef";
      }
      break;
    case "after_fmt_undef":
      if (matches("endif", /^$/u)) return "between_sections";
      break;
    case "between_sections":
      if (matches("ifdef", /^DEF$/u)) return "before_fallback";
      break;
    case "before_fallback":
      if (matches("ifndef", /^def$/u)) return "fallback_body";
      break;
    case "fallback_body":
      if (matches("define", quickJsFallbackDefinition())) {
        return "after_fallback_define";
      }
      break;
    case "after_fallback_define":
      if (matches("endif", /^$/u)) return "final_long";
      break;
    case "temporaries":
      if (
        matches("if", /^SHORT_OPCODES$/u) &&
        temporaryEntries.length === 19 &&
        entries.length === 178
      ) {
        return "short";
      }
      break;
    case "short":
      if (matches("endif", /^$/u) && entries.length === 244) {
        return "after_short";
      }
      break;
    case "after_short":
      if (matches("undef", /^DEF$/u)) return "after_def_undef";
      break;
    case "after_def_undef":
      if (matches("undef", /^def$/u)) return "after_def_alias_undef";
      break;
    case "after_def_alias_undef":
      if (matches("endif", /^$/u)) return "done";
      break;
    default:
      break;
  }

  fail(
    `${path}:${line}: unexpected #${directive}` +
      (arguments_.length === 0 ? "" : ` ${arguments_}`) +
      ` in ${state} section`,
  );
}

function quickJsFallbackDefinition() {
  return /^def\(\s*id\s*,\s*size\s*,\s*n_pop\s*,\s*n_push\s*,\s*f\s*\)\s+DEF\(\s*id\s*,\s*size\s*,\s*n_pop\s*,\s*n_push\s*,\s*f\s*\)$/u;
}

function parseCInteger(text, path, line) {
  const value = Number.parseInt(text, 10);
  if (!Number.isSafeInteger(value) || value < 0 || value > 255) {
    fail(`${path}:${line}: opcode descriptor integer is outside u8 range`);
  }
  return value;
}

function validateQuickJsSource(formats, entries, temporaryEntries, path) {
  const names = formats.map((entry) => entry.name);
  if (!arraysEqual(names, quickJsFormatNames)) {
    fail(
      `${path}: expected the pinned ${quickJsFormatNames.length} FMT entries ` +
        `in release order, found ${JSON.stringify(names)}`,
    );
  }
  assertUniqueNames(formats, path, "FMT");
  validateWireCatalog(entries, path);

  if (temporaryEntries.length !== expectedTemporaryDescriptors.length) {
    fail(
      `${path}: expected ${expectedTemporaryDescriptors.length} lowercase ` +
        `temporary descriptors, found ${temporaryEntries.length}`,
    );
  }
  assertUniqueNames(temporaryEntries, path, "temporary opcode");
  const finalNames = new Set(entries.map((entry) => entry.name));
  for (const entry of temporaryEntries) {
    if (finalNames.has(entry.name)) {
      fail(`${path}:${entry.line}: temporary opcode ${entry.name} is also final`);
    }
  }

  const temporaryFailures = compareEntries(
    expectedTemporaryDescriptors,
    temporaryEntries,
    "temporary opcode",
  );
  if (temporaryFailures.length > 0) {
    fail(`${path}: ${temporaryFailures.join("; ")}`);
  }

  const nopLine = entries[177].line;
  const firstShortLine = entries[178].line;
  if (
    temporaryEntries.some(
      (entry) => entry.line <= nopLine || entry.line >= firstShortLine,
    )
  ) {
    fail(`${path}: temporary descriptors must occur between nop and short opcodes`);
  }
}

function validateWireCatalog(entries, path) {
  if (entries.length !== 244) {
    fail(`${path}: expected 244 final wire opcodes, found ${entries.length}`);
  }
  assertUniqueNames(entries, path, "final opcode");
  for (const entry of entries) {
    if (!quickJsFormatNames.includes(entry.format)) {
      fail(`${path}: opcode ${entry.id} ${entry.name} has unknown ${entry.format} format`);
    }
  }

  if (entries[0].name !== "invalid") {
    fail(`${path}: opcode 0 must be invalid`);
  }
  if (entries[177].name !== "nop") {
    fail(`${path}: opcode 177 must be nop`);
  }
  if (entries[178].name !== "push_minus1") {
    fail(`${path}: short opcode range must begin with push_minus1 at 178`);
  }
  if (entries[243].name !== "typeof_is_function") {
    fail(`${path}: short opcode range must end with typeof_is_function at 243`);
  }

  const atomEntries = entries.filter((entry) => atomOperandOffset(entry.format) !== null);
  const atomIds = atomEntries.map((entry) => entry.id);
  if (!arraysEqual(atomIds, atomBearingOpcodeIds)) {
    fail(
      `${path}: atom-bearing opcode IDs are ${JSON.stringify(atomIds)}, ` +
        `expected ${JSON.stringify(atomBearingOpcodeIds)}`,
    );
  }
  for (const entry of atomEntries) {
    const offset = atomOperandOffset(entry.format);
    if (offset !== 1 || entry.size < offset + 4) {
      fail(
        `${path}: opcode ${entry.id} ${entry.name} must contain its u32 atom ` +
          `operand at +1`,
      );
    }
  }
}

function atomOperandOffset(format) {
  return format === "atom" || format.startsWith("atom_") ? 1 : null;
}

function assertUniqueNames(entries, path, label) {
  const seen = new Map();
  for (const entry of entries) {
    const previous = seen.get(entry.name);
    if (previous !== undefined) {
      const location = entry.line === undefined ? "" : `:${entry.line}`;
      fail(
        `${path}${location}: duplicate ${label} ${entry.name}` +
          (previous === null ? "" : ` (first at line ${previous})`),
      );
    }
    seen.set(entry.name, entry.line ?? null);
  }
}

function parseRustManifest(source, path) {
  const manifest = inspectRustManifest(
    source,
    path,
    "opcode",
    allowedRustManifestAttributes,
  );
  const { uncommented, structural } = manifest;

  const count = parseRustCount(manifest);
  const arrayIndex = findTopLevelConst(
    manifest,
    "PINNED_OPCODE_INFO",
    ["#[rustfmt::skip]"],
  );
  const arrayStart =
    /^const\s+PINNED_OPCODE_INFO\s*:\s*\[\s*PinnedOpcodeInfo\s*;\s*PINNED_OPCODE_COUNT\s*\]\s*=\s*\[/u;
  const startMatch = arrayStart.exec(structural.slice(arrayIndex));
  if (startMatch === null) {
    fail(`${path}: PINNED_OPCODE_INFO must be one direct fixed array`);
  }
  const open = arrayIndex + startMatch[0].lastIndexOf("[");
  const end = findMatchingArrayEnd(manifest, open, "PINNED_OPCODE_INFO");
  const afterArray = structural.slice(end + 1).match(/^\s*/u)?.[0].length ?? 0;
  if (structural[end + 1 + afterArray] !== ";") {
    fail(`${path}: PINNED_OPCODE_INFO array must end with a semicolon`);
  }
  const entries = parseRustOpcodeArray(
    uncommented.slice(open + 1, end),
    path,
  );
  return { count, entries };
}

function parseRustCount(manifest) {
  const { path, structural } = manifest;
  const index = findTopLevelConst(
    manifest,
    "PINNED_OPCODE_COUNT",
    [],
  );
  const expression =
    /^const\s+PINNED_OPCODE_COUNT\s*:\s*usize\s*=\s*([0-9][0-9_]*)\s*;/u;
  const match = expression.exec(structural.slice(index));
  if (match === null) {
    fail(`${path}: PINNED_OPCODE_COUNT must be one direct numeric usize constant`);
  }
  return parseRustInteger(match[1], `${path}:PINNED_OPCODE_COUNT`);
}

function parseRustOpcodeArray(body, path) {
  const entries = [];
  let offset = 0;
  const entry =
    /^PinnedOpcodeInfo\s*::\s*new\s*\(\s*("(?:\\.|[^"\\])*")\s*,\s*([0-9][0-9_]*)\s*,\s*([0-9][0-9_]*)\s*,\s*([0-9][0-9_]*)\s*,\s*OpcodeFormat\s*::\s*([A-Za-z_][A-Za-z0-9_]*)\s*\)\s*,/u;

  while (offset < body.length) {
    const whitespace = /^\s+/u.exec(body.slice(offset));
    if (whitespace !== null) {
      offset += whitespace[0].length;
      continue;
    }
    if (offset === body.length) {
      break;
    }
    const match = entry.exec(body.slice(offset));
    if (match === null) {
      fail(`${path}: unsupported token in PINNED_OPCODE_INFO at byte ${offset}`);
    }
    const format = rustFormatNames.get(match[5]);
    if (format === undefined) {
      fail(`${path}: unsupported OpcodeFormat::${match[5]} in pinned manifest`);
    }
    entries.push({
      format,
      id: entries.length,
      nPop: parseRustInteger(match[3], `${path} opcode n_pop`),
      nPush: parseRustInteger(match[4], `${path} opcode n_push`),
      name: decodeQuotedString(match[1], `${path} opcode name`),
      size: parseRustInteger(match[2], `${path} opcode size`),
    });
    offset += match[0].length;
  }
  return entries;
}

function parseRustInteger(text, location) {
  const value = Number.parseInt(text.replaceAll("_", ""), 10);
  if (!Number.isSafeInteger(value) || value < 0 || value > 255) {
    fail(`${location}: integer is outside u8 range`);
  }
  return value;
}

function compareCatalogs(expected, actual) {
  const failures = [];
  if (actual.count !== expected.length) {
    failures.push(
      `PINNED_OPCODE_COUNT is ${actual.count}, QuickJS defines ${expected.length}`,
    );
  }
  if (actual.entries.length !== actual.count) {
    failures.push(
      `PINNED_OPCODE_INFO has ${actual.entries.length} entries, ` +
        `PINNED_OPCODE_COUNT is ${actual.count}`,
    );
  }
  failures.push(...compareEntries(expected, actual.entries, "opcode"));
  return failures.slice(0, 21);
}

function compareEntries(expected, actual, label) {
  const failures = [];
  const comparedLength = Math.min(expected.length, actual.length);
  for (let index = 0; index < comparedLength; index += 1) {
    const wanted = expected[index];
    const found = actual[index];
    for (const field of ["id", "name", "size", "nPop", "nPush", "format"]) {
      if (found[field] !== wanted[field]) {
        failures.push(
          `${label} ${index} ${field} is ${JSON.stringify(found[field])}, ` +
            `expected ${JSON.stringify(wanted[field])}`,
        );
        if (failures.length >= 20) {
          failures.push("additional manifest differences omitted");
          return failures;
        }
      }
    }
  }
  if (actual.length !== expected.length) {
    failures.push(
      `${label} catalog has ${actual.length} entries, expected ${expected.length}`,
    );
  }
  return failures;
}

function manifestSha256(entries) {
  const canonical = entries
    .map(
      (entry) =>
        `${entry.id}\t${entry.name}\t${entry.size}\t${entry.nPop}\t` +
        `${entry.nPush}\t${entry.format}\n`,
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

function runQuickJsParserSelfTests() {
  const valid = selfTestQuickJsSource();
  const parsed = parseQuickJsOpcodes(valid, "self-test valid QuickJS source");
  if (parsed.entries.length !== 244 || parsed.temporaryEntries.length !== 19) {
    fail("self-test QuickJS source did not preserve final/temporary boundary");
  }

  const withDecoys =
    `/* #if 0\nFMT(decoy)\nDEF(decoy, 1, 0, 0, none)\n` +
    `#define DEF(id, size, n_pop, n_push, f)\n#endif */\n` +
    `// #undef FMT\n` +
    valid;
  const decoyParsed = parseQuickJsOpcodes(
    withDecoys,
    "self-test QuickJS comment/macro decoy",
  );
  if (compareEntries(parsed.entries, decoyParsed.entries, "opcode").length !== 0) {
    fail("self-test QuickJS comment/directive decoy changed the wire catalog");
  }

  expectGateFailure("inactive QuickJS short branch", () =>
    parseQuickJsOpcodes(
      valid.replace("#if SHORT_OPCODES", "#if 0"),
      "self-test inactive QuickJS short branch",
    ),
  );
  expectGateFailure("QuickJS C line continuation", () =>
    parseQuickJsOpcodes(
      `// hidden \\\n${valid}`,
      "self-test QuickJS C line continuation",
    ),
  );
  expectGateFailure("QuickJS C bare-CR continuation", () =>
    parseQuickJsOpcodes(
      `// hidden \\\r${valid}`,
      "self-test QuickJS C bare-CR continuation",
    ),
  );
  expectGateFailure("QuickJS C trigraph", () =>
    parseQuickJsOpcodes(
      `// hidden ??/\n${valid}`,
      "self-test QuickJS C trigraph",
    ),
  );
  expectGateFailure("disabled QuickJS final descriptors", () =>
    parseQuickJsOpcodes(
      valid.replace(
        "DEF(invalid, 1, 0, 0, none)",
        `#undef DEF\n` +
          `#define DEF(id, size, n_pop, n_push, f)\n` +
          `DEF(invalid, 1, 0, 0, none)`,
      ),
      "self-test disabled QuickJS final descriptors",
    ),
  );
  expectGateFailure("empty QuickJS def fallback", () =>
    parseQuickJsOpcodes(
      valid.replace(
        "#define def(id, size, n_pop, n_push, f) DEF(id, size, n_pop, n_push, f)",
        "#define def(id, size, n_pop, n_push, f)",
      ),
      "self-test empty QuickJS def fallback",
    ),
  );
  expectGateFailure("QuickJS alternate short branch", () =>
    parseQuickJsOpcodes(
      valid.replace(
        "#if SHORT_OPCODES\n",
        "#if SHORT_OPCODES\n#else\n",
      ),
      "self-test QuickJS alternate short branch",
    ),
  );
  expectGateFailure("QuickJS active macro decoy", () =>
    parseQuickJsOpcodes(
      `#define DECOY() DEF(decoy, 1, 0, 0, none)\n${valid}`,
      "self-test QuickJS active macro decoy",
    ),
  );

  expectGateFailure("malformed QuickJS descriptor", () =>
    parseQuickJsOpcodes(
      valid.replace("DEF(invalid, 1, 0, 0, none)", "DEF(invalid, nope)"),
      "self-test malformed QuickJS descriptor",
    ),
  );
  expectGateFailure("duplicate QuickJS descriptor", () =>
    parseQuickJsOpcodes(
      valid.replace(
        "DEF(invalid, 1, 0, 0, none)",
        "DEF(invalid, 1, 0, 0, none)\nDEF(invalid, 1, 0, 0, none)",
      ),
      "self-test duplicate QuickJS descriptor",
    ),
  );
  expectGateFailure("missing QuickJS temporary descriptor", () =>
    parseQuickJsOpcodes(
      valid.replace("def(line_num, 5, 0, 0, u32)\n", ""),
      "self-test missing QuickJS temporary descriptor",
    ),
  );
  expectGateFailure("duplicate QuickJS format", () =>
    parseQuickJsOpcodes(
      valid.replace("FMT(none)\n", "FMT(none)\nFMT(none)\n"),
      "self-test duplicate QuickJS format",
    ),
  );
}

function selfTestQuickJsSource() {
  const lines = ["#ifdef FMT"];
  lines.push(...quickJsFormatNames.map((format) => `FMT(${format})`));
  lines.push("#undef FMT", "#endif", "#ifdef DEF", "#ifndef def");
  lines.push(
    "#define def(id, size, n_pop, n_push, f) " +
      "DEF(id, size, n_pop, n_push, f)",
    "#endif",
  );
  for (let id = 0; id <= 177; id += 1) {
    lines.push(selfTestQuickJsDescriptor("DEF", id));
  }
  for (let index = 0; index < expectedTemporaryDescriptors.length; index += 1) {
    const descriptor = expectedTemporaryDescriptors[index];
    lines.push(
      `def(${descriptor.name}, ` +
        `${descriptor.size}, ${descriptor.nPop}, ${descriptor.nPush}, ` +
        `${descriptor.format})`,
    );
  }
  lines.push("#if SHORT_OPCODES");
  for (let id = 178; id < 244; id += 1) {
    lines.push(selfTestQuickJsDescriptor("DEF", id));
  }
  lines.push("#endif", "#undef DEF", "#undef def", "#endif");
  return `${lines.join("\n")}\n`;
}

function selfTestQuickJsDescriptor(macro, id) {
  const specialNames = new Map([
    [0, "invalid"],
    [177, "nop"],
    [178, "push_minus1"],
    [243, "typeof_is_function"],
  ]);
  const name = specialNames.get(id) ?? `opcode_${id}`;
  const format = atomBearingOpcodeIds.includes(id) ? "atom" : "none";
  const size = format === "atom" ? 5 : 1;
  return `${macro}(${name}, ${size}, 0, 0, ${format})`;
}

function runRustParserSelfTests() {
  const expected = selfTestRustEntries("valid");
  const valid = selfTestRustManifest(expected);
  const wrong = selfTestRustManifest(selfTestRustEntries("wrong"));
  const parsed = parseRustManifest(valid, "self-test valid Rust manifest");
  if (compareEntries(expected, parsed.entries, "opcode").length !== 0) {
    fail("self-test valid Rust manifest did not round-trip");
  }

  const commentDecoy = parseRustManifest(
    `/* ${valid} */\n${wrong}`,
    "self-test Rust comment decoy",
  );
  if (compareEntries(expected, commentDecoy.entries, "opcode").length === 0) {
    fail("self-test Rust comment decoy hid the active wrong manifest");
  }

  const stringDecoy = parseRustManifest(
    `const DECOY: &str = r###"${valid}"###;\n${wrong}`,
    "self-test Rust string decoy",
  );
  if (compareEntries(expected, stringDecoy.entries, "opcode").length === 0) {
    fail("self-test Rust string decoy hid the active wrong manifest");
  }

  expectGateFailure("Rust conditional decoy", () =>
    parseRustManifest(
      `#[cfg(any())]\n${valid}\n${wrong}`,
      "self-test Rust conditional decoy",
    ),
  );
  expectGateFailure("Rust duplicate decoy", () =>
    parseRustManifest(`${valid}\n${wrong}`, "self-test Rust duplicate decoy"),
  );
  expectGateFailure("Rust nested decoy", () =>
    parseRustManifest(
      `mod decoy {\n${valid}\n}\n${wrong}`,
      "self-test Rust nested decoy",
    ),
  );
  expectGateFailure("Rust parenthesized macro wrapper", () =>
    parseRustManifest(
      `passthrough!(\n${valid}\n);`,
      "self-test Rust parenthesized macro wrapper",
    ),
  );
  expectGateFailure("Rust bracket macro wrapper", () =>
    parseRustManifest(
      `passthrough![\n${valid}\n];`,
      "self-test Rust bracket macro wrapper",
    ),
  );
  expectGateFailure("Rust count attribute decoy", () =>
    parseRustManifest(
      valid.replace(
        "const PINNED_OPCODE_COUNT",
        "#[evil]\nconst PINNED_OPCODE_COUNT",
      ),
      "self-test Rust count attribute decoy",
    ),
  );
  expectGateFailure("Rust catalog attribute decoy", () =>
    parseRustManifest(
      valid.replace("#[rustfmt::skip]", "#[evil]\n#[rustfmt::skip]"),
      "self-test Rust catalog attribute decoy",
    ),
  );
  expectGateFailure("Rust include decoy", () =>
    parseRustManifest(
      `const DECOY: &[u8] = include_bytes!("decoy");\n${wrong}`,
      "self-test Rust include decoy",
    ),
  );
  expectGateFailure("Rust macro decoy", () =>
    parseRustManifest(
      `macro_rules! decoy { () => { ${valid} } }\n${wrong}`,
      "self-test Rust macro decoy",
    ),
  );
  expectGateFailure("Rust production tail after tests", () =>
    parseRustManifest(
      `${valid}\n#[cfg(test)]\nmod tests {}\n` +
        `const ACTIVE_PRODUCTION_TAIL: usize = 1;\n`,
      "self-test Rust production tail after tests",
    ),
  );
  expectGateFailure("Rust unrelated production attribute", () =>
    parseRustManifest(
      `#[evil]\nstruct Decoy;\n${valid}`,
      "self-test Rust unrelated production attribute",
    ),
  );
  expectGateFailure("Rust derive-shadowing import", () =>
    parseRustManifest(
      `use evil::Clone;\n${valid}`,
      "self-test Rust derive-shadowing import",
    ),
  );
  expectGateFailure("Rust qualified function-like macro", () =>
    parseRustManifest(
      `evil::replace_manifest!();\n${valid}`,
      "self-test Rust qualified function-like macro",
    ),
  );
  expectGateFailure("Rust Unicode function-like macro", () =>
    parseRustManifest(
      `evil::\u6076\u610f!();\n${valid}`,
      "self-test Rust Unicode function-like macro",
    ),
  );
  expectGateFailure("Rust production unary exclamation", () =>
    parseRustManifest(
      `const DECOY: bool = !(false);\n${valid}`,
      "self-test Rust production unary exclamation",
    ),
  );
  expectGateFailure("Rust unsupported manifest token", () =>
    parseRustManifest(
      valid.replace("PinnedOpcodeInfo::new", "make_opcode"),
      "self-test Rust unsupported token",
    ),
  );
}

function selfTestRustEntries(prefix) {
  return [
    { id: 0, name: `${prefix}_zero`, size: 1, nPop: 0, nPush: 0, format: "none" },
    { id: 1, name: `${prefix}_one`, size: 5, nPop: 1, nPush: 1, format: "atom" },
    { id: 2, name: `${prefix}_two`, size: 2, nPop: 0, nPush: 1, format: "u8" },
  ];
}

function selfTestRustManifest(entries) {
  const formatVariants = new Map(quickJsFormats);
  return `
const PINNED_OPCODE_COUNT: usize = ${entries.length};
#[rustfmt::skip]
const PINNED_OPCODE_INFO: [PinnedOpcodeInfo; PINNED_OPCODE_COUNT] = [
${entries
  .map(
    (entry) =>
      `    PinnedOpcodeInfo::new(${JSON.stringify(entry.name)}, ` +
      `${entry.size}, ${entry.nPop}, ${entry.nPush}, ` +
      `OpcodeFormat::${formatVariants.get(entry.format)}),`,
  )
  .join("\n")}
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

function arraysEqual(left, right) {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}
