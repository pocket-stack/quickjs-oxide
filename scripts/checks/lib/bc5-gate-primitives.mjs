import { lstatSync, readFileSync } from "node:fs";

export class GateError extends Error {
  constructor(message, status = 1) {
    super(message);
    this.name = "GateError";
    this.status = status;
  }
}

export function fail(message, status = 1) {
  throw new GateError(message, status);
}

export function readRegularFile(path, label) {
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

/**
 * Blank C comments and literals without changing byte or line offsets.
 *
 * `uncommented` retains literal spellings for domain parsers. `structural`
 * additionally blanks literals so directives and delimiters cannot be forged
 * from quoted text.
 */
export function inspectCSource(source, path) {
  if (/\\(?:\r\n?|\n)/u.test(source)) {
    fail(`${path}: C line continuations are not admitted by this gate`);
  }
  if (/\?\?[=\/'()!<>-]/u.test(source)) {
    fail(`${path}: C trigraphs are not admitted by this gate`);
  }
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
      const end = source.indexOf("*/", index + 2);
      if (end === -1) {
        fail(`${path}: unterminated C block comment`);
      }
      const stop = end + 2;
      blankRange(uncommented, index, stop);
      blankRange(structural, index, stop);
      index = stop;
      continue;
    }

    const literal = /^(?:u8|[uUL])?(["'])/u.exec(source.slice(index));
    if (literal !== null) {
      const quote = literal[1];
      let cursor = index + literal[0].length;
      let escaped = false;
      while (cursor < source.length) {
        const character = source[cursor];
        if (!escaped && character === quote) {
          cursor += 1;
          break;
        }
        if (!escaped && (character === "\n" || character === "\r")) {
          fail(`${path}: newline in C literal`);
        }
        escaped = !escaped && character === "\\";
        if (character !== "\\") {
          escaped = false;
        }
        cursor += 1;
      }
      if (cursor > source.length || source[cursor - 1] !== quote) {
        fail(`${path}: unterminated C literal`);
      }
      blankRange(structural, index, cursor);
      index = cursor;
      continue;
    }

    index += 1;
  }

  return {
    uncommented: uncommented.join(""),
    structural: structural.join(""),
  };
}

/**
 * Authenticate the production portion of a Rust pinned-manifest source.
 *
 * The sole admitted conditional-compilation boundary is one exact, top-level
 * `#[cfg(test)] mod tests { ... }` at EOF. The returned strings retain source
 * offsets but exclude that test module.
 */
export function inspectRustManifest(
  source,
  path,
  manifestKind,
  allowedProductionAttributes = [],
  allowedProductionUses = [],
) {
  const lexed = lexRust(source, path);
  const completeDepth = computeDelimiterDepth(
    lexed.structural,
    path,
    manifestKind,
  );
  const testModules = [
    ...lexed.structural.matchAll(
      /#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*mod\s+tests\b/gu,
    ),
  ].filter((match) => completeDepth[match.index] === 0);

  if (testModules.length > 1) {
    fail(`${path}: expected at most one top-level cfg(test) tests module`);
  }

  let productionEnd = source.length;
  if (testModules.length === 1) {
    const testModule = testModules[0];
    const lineStart =
      lexed.structural.lastIndexOf("\n", testModule.index - 1) + 1;
    if (lexed.structural.slice(lineStart, testModule.index).trim() !== "") {
      fail(`${path}: cfg(test) tests module must begin at item boundary`);
    }
    const before = skipWhitespaceBackward(lexed.structural, testModule.index);
    if (before > 0 && lexed.structural[before - 1] === "]") {
      fail(`${path}: cfg(test) tests module must have no additional attributes`);
    }

    const afterName = skipWhitespaceForward(
      lexed.structural,
      testModule.index + testModule[0].length,
    );
    if (lexed.structural[afterName] !== "{") {
      fail(`${path}: cfg(test) tests module must be an inline module`);
    }
    const close = findTopLevelModuleClose(
      lexed.structural,
      completeDepth,
      afterName,
      path,
    );
    if (lexed.structural.slice(close + 1).trim() !== "") {
      fail(`${path}: active tokens follow the cfg(test) tests module`);
    }
    productionEnd = testModule.index;
  }

  const uncommented = lexed.uncommented.slice(0, productionEnd);
  const structural = lexed.structural.slice(0, productionEnd);
  const delimiterDepth = computeDelimiterDepth(structural, path, manifestKind);
  rejectManifestIndirection(structural, path, manifestKind);
  assertProductionAttributes(
    structural,
    path,
    manifestKind,
    allowedProductionAttributes,
  );
  assertProductionUses(
    structural,
    delimiterDepth,
    path,
    manifestKind,
    allowedProductionUses,
  );
  return {
    delimiterDepth,
    manifestKind,
    path,
    structural,
    uncommented,
  };
}

/**
 * Locate exactly one module-level const and authenticate its item attributes.
 */
export function findTopLevelConst(manifest, name, allowedAttributes = []) {
  const { delimiterDepth, path, structural } = manifest;
  const expression = new RegExp(`\\bconst\\s+${name}\\b`, "gu");
  const matches = [...structural.matchAll(expression)];
  if (matches.length !== 1) {
    fail(`${path}: expected exactly one ${name} const, found ${matches.length}`);
  }
  const index = matches[0].index;
  if (delimiterDepth[index] !== 0) {
    fail(`${path}: ${name} must be defined at module top level`);
  }
  assertRustItemAttributes(
    structural,
    index,
    path,
    name,
    allowedAttributes,
  );
  return index;
}

export function findMatchingArrayEnd(manifest, open, name) {
  const { path, structural } = manifest;
  if (structural[open] !== "[") {
    fail(`${path}: ${name} array does not begin at byte ${open}`);
  }
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
  fail(`${path}: unterminated ${name} array`);
}

export function decodeQuotedString(literal, location) {
  try {
    return JSON.parse(literal);
  } catch (error) {
    fail(`${location}: unsupported string literal ${literal}: ${error.message}`);
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

    const ordinaryPrefix =
      source[index] === '"'
        ? 0
        : source[index] === "b" && source[index + 1] === '"'
          ? 1
          : null;
    if (ordinaryPrefix !== null) {
      let cursor = index + ordinaryPrefix + 1;
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

    const characterLiteral = /^(?:b)?'(?:\\.|[^'\\\r\n])'/u.exec(
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

function computeDelimiterDepth(structural, path, manifestKind) {
  const depthAt = new Uint32Array(structural.length + 1);
  const stack = [];
  const closing = new Map([
    ["}", "{"],
    [")", "("],
    ["]", "["],
  ]);
  for (let index = 0; index < structural.length; index += 1) {
    depthAt[index] = stack.length;
    const token = structural[index];
    if (token === "{" || token === "(" || token === "[") {
      stack.push(token);
    } else if (closing.has(token)) {
      const expected = closing.get(token);
      const found = stack.pop();
      if (found !== expected) {
        fail(
          `${path}: unmatched or misnested ${token} in ${manifestKind} manifest`,
        );
      }
    }
  }
  depthAt[structural.length] = stack.length;
  if (stack.length !== 0) {
    fail(`${path}: unmatched ${stack.at(-1)} in ${manifestKind} manifest`);
  }
  return depthAt;
}

function findTopLevelModuleClose(structural, depthAt, open, path) {
  if (depthAt[open] !== 0 || structural[open] !== "{") {
    fail(`${path}: cfg(test) tests module is not top-level`);
  }
  for (let index = open + 1; index < structural.length; index += 1) {
    if (structural[index] === "}" && depthAt[index] === 1) {
      return index;
    }
  }
  fail(`${path}: unterminated cfg(test) tests module`);
}

function rejectManifestIndirection(structural, path, manifestKind) {
  const forbidden = [
    [/#\s*!?\s*\[\s*cfg(?:_attr)?\b/u, "conditional compilation"],
    [
      /!/u,
      "exclamation token (macros and unary operators are not admitted)",
    ],
    [/\bmacro\s+[A-Za-z_][A-Za-z0-9_]*/u, "macro definition"],
    [/\bextern\s+crate\b/u, "extern crate import"],
  ];
  for (const [pattern, label] of forbidden) {
    if (pattern.test(structural)) {
      fail(
        `${path}: ${label} is not allowed in the production ` +
          `${manifestKind} manifest`,
      );
    }
  }
}

function assertProductionUses(
  structural,
  delimiterDepth,
  path,
  manifestKind,
  allowedUses,
) {
  const uses = [];
  const useKeyword = /\buse\b/gu;
  let match;
  while ((match = useKeyword.exec(structural)) !== null) {
    if (delimiterDepth[match.index] !== 0) {
      fail(`${path}: production ${manifestKind} use must be module-level`);
    }
    const lineStart = structural.lastIndexOf("\n", match.index - 1) + 1;
    if (structural.slice(lineStart, match.index).trim() !== "") {
      fail(`${path}: unsupported token before production ${manifestKind} use`);
    }
    const end = structural.indexOf(";", match.index);
    if (end === -1) {
      fail(`${path}: unterminated production ${manifestKind} use`);
    }
    uses.push(
      structural.slice(match.index, end + 1).replaceAll(/\s/gu, ""),
    );
    useKeyword.lastIndex = end + 1;
  }

  const expected = allowedUses.map((statement) =>
    statement.replaceAll(/\s/gu, ""),
  );
  if (!arraysEqual(uses, expected)) {
    fail(
      `${path}: production ${manifestKind} uses are ${JSON.stringify(uses)}, ` +
        `expected ${JSON.stringify(expected)}`,
    );
  }
}

function assertProductionAttributes(
  structural,
  path,
  manifestKind,
  allowedAttributes,
) {
  const allowed = new Set(
    allowedAttributes.map((attribute) => attribute.replaceAll(/\s/gu, "")),
  );
  const attribute = /#\s*(!?)\s*\[/gu;
  let match;
  while ((match = attribute.exec(structural)) !== null) {
    const open = structural.indexOf("[", match.index);
    const close = findClosingBracketForward(
      structural,
      open,
      path,
      `${manifestKind} attribute`,
    );
    const spelling = structural
      .slice(match.index, close + 1)
      .replaceAll(/\s/gu, "");
    if (match[1] === "!" || !allowed.has(spelling)) {
      fail(`${path}: unsupported production ${manifestKind} attribute ${spelling}`);
    }
    attribute.lastIndex = close + 1;
  }
}

function assertRustItemAttributes(
  structural,
  constIndex,
  path,
  name,
  expectedAttributes,
) {
  const attributes = [];
  const beforeConst = structural.slice(0, constIndex);
  const visibility = /\bpub(?:\s*\([^()]*\))?\s*$/u.exec(beforeConst);
  let cursor = visibility?.index ?? constIndex;
  while (true) {
    const attributeEnd = skipWhitespaceBackward(structural, cursor);
    if (structural[attributeEnd - 1] !== "]") {
      break;
    }
    const close = attributeEnd - 1;
    const open = findOpeningBracketBackward(structural, close, path, name);
    const hash = skipWhitespaceBackward(structural, open);
    if (structural[hash - 1] !== "#") {
      break;
    }
    const start = hash - 1;
    attributes.unshift(
      structural.slice(start, close + 1).replaceAll(/\s/gu, ""),
    );
    cursor = start;
  }

  const lineStart = structural.lastIndexOf("\n", cursor - 1) + 1;
  if (structural.slice(lineStart, cursor).trim() !== "") {
    fail(`${path}: unsupported token before ${name} const`);
  }

  const expected = expectedAttributes.map((attribute) =>
    attribute.replaceAll(/\s/gu, ""),
  );
  if (!arraysEqual(attributes, expected)) {
    fail(
      `${path}: ${name} attributes are ${JSON.stringify(attributes)}, ` +
        `expected ${JSON.stringify(expected)}`,
    );
  }
}

function findOpeningBracketBackward(source, close, path, name) {
  let depth = 1;
  for (let index = close - 1; index >= 0; index -= 1) {
    if (source[index] === "]") {
      depth += 1;
    } else if (source[index] === "[") {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }
  fail(`${path}: unmatched ] before ${name} const`);
}

function findClosingBracketForward(source, open, path, name) {
  let depth = 1;
  for (let index = open + 1; index < source.length; index += 1) {
    if (source[index] === "[") {
      depth += 1;
    } else if (source[index] === "]") {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }
  fail(`${path}: unmatched [ in ${name}`);
}

function skipWhitespaceBackward(source, end) {
  let cursor = end;
  while (cursor > 0 && /\s/u.test(source[cursor - 1])) {
    cursor -= 1;
  }
  return cursor;
}

function skipWhitespaceForward(source, start) {
  let cursor = start;
  while (cursor < source.length && /\s/u.test(source[cursor])) {
    cursor += 1;
  }
  return cursor;
}

function blankRange(characters, start, end) {
  for (let index = start; index < end; index += 1) {
    if (characters[index] !== "\n" && characters[index] !== "\r") {
      characters[index] = " ";
    }
  }
}

function arraysEqual(left, right) {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}
