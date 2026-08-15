#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { basename, dirname, join, relative, resolve, sep } from "node:path";

import {
  admissionRecord,
  assertAdmissionGroup,
  renderAdmissionRows,
} from "./test262-admission-data.mjs";

const root = resolve(import.meta.dirname, "..");
const checkedSource = join(root, "target/oracle/quickjs-2026-06-04");
const checkedSuite = join(checkedSource, "test262");
const checkedRunner = join(checkedSource, "run-test262");
const checkedConfig = join(checkedSource, "test262.conf");
const checkedAdmissions = join(root, "dev-support/test262/admissions.tsv");
const args = process.argv.slice(2);

function option(name, fallback) {
  const indexes = args.flatMap((value, index) => (value === name ? [index] : []));
  assert(indexes.length <= 1, `duplicate ${name}`);
  if (indexes.length === 0) return fallback;
  const value = args[indexes[0] + 1];
  assert(value && !value.startsWith("--"), `${name} requires a value`);
  return resolve(value);
}

const suite = option("--suite", checkedSuite);
const quickjsRunner = option("--quickjs-runner", checkedRunner);
const quickjsConfig = option("--quickjs-config", checkedConfig);
const selectedModes = ["--admissions", "--check-current"].filter((mode) =>
  args.includes(mode),
);
assert(selectedModes.length <= 1, "select at most one output/check mode");
const mode = selectedModes[0] ?? "--check-current";

const valueOptions = new Set(["--suite", "--quickjs-runner", "--quickjs-config"]);
const flagOptions = new Set(["--admissions", "--check-current"]);
for (let index = 0; index < args.length; index += 1) {
  const argument = args[index];
  if (valueOptions.has(argument)) {
    index += 1;
  } else {
    assert(flagOptions.has(argument), `unknown option: ${argument}`);
  }
}

assert(existsSync(join(suite, "test")), `missing Test262 suite: ${suite}`);
assert(existsSync(quickjsRunner), `missing pinned QuickJS run-test262: ${quickjsRunner}`);
assert(existsSync(quickjsConfig), `missing pinned QuickJS Test262 config: ${quickjsConfig}`);

const cohort = "test/language/module-code/top-level-await/syntax";
const admissionGroup = "module-var-lexical-shadow-a";
const suffixes = [
  "array-literal",
  "func-expression",
  "identifier",
  "literal-number",
  "literal-string",
  "nested",
  "new-expr",
  "null",
  "obj-literal",
  "regexp",
  "template-literal",
  "this",
];
const families = [
  {
    prefix: "for-await-await-expr",
    features: ["top-level-await", "async-iteration"],
    requiredBodyFragments: [
      "\nvar binding;\n",
      "for await (var binding of",
      "for await (let binding of",
    ],
    hashes: [
      "d9e4b23acc1185a0d48f36816fdaef06962382a75945e16d8b77f75cd099c0c7",
      "e4aaf4dfd54597bc67c38849765913cf6bd7083148a141c7088950a067b4c4a2",
      "cd70ac7166addd7726b206ae75eae3c30e30ddd1e05eb987dc88e3d0e7aa6ad5",
      "03c5e10638875eaecc99259daeb810165a573787c3306c64879a4f003fcc1f49",
      "1a31051e9a1cfbdda4d8df063260f70b9c9452a378a404a685a3b0976b8c0d39",
      "5c67f98c4076fa3516d30a630d679cc577995e38c6b87d7acfd7fcc75b45d816",
      "c0393a5611cf82f3e24d1b82d530092ea10bb2dc5969da72cba1a0610b2e24c9",
      "a050cb89f9c168c31d898c5e71df64c90c06e0f9dc6951f829d9bdb2fa0f2966",
      "c210d9242c45358904d886d33ace56f60e4e4a0310f8daa8524961ff2964c5d4",
      "90fa953b3a45c0072c478ba875eda3c07bee81bd377abfe6b1de03d486d667b1",
      "f5b43eab88413e2532d68956a2985a3c9c8f91ec3555ea6b5aca7007b3178792",
      "9b9c0e48966d801596dc2b1cc34f61e67611a31823c385b207bdfdd8d858bbf7",
    ],
  },
  {
    prefix: "for-await-expr",
    features: ["top-level-await"],
    requiredBodyFragments: ["for ( var binding;", "for ( let binding;"],
    hashes: [
      "8fbfc5ed841580928aad8833b6b444fa608aa6226b26bf6e7a9659d06a6d6602",
      "d81ae36fb7b485324bd0ad909483c2b473ad84228aa5d690a71c085b822f6f8f",
      "39e1fa2012652b434941bd26f579c632ff0e49647ef443f83bf295a2362ecd5e",
      "9e773e10fa2ba58c46796c219ef97b586c67776768dc39fce95cb0127caf390e",
      "753f0db0230f2d491c78722112840d8d580e774f58606ff5c00a77e5f00ecec5",
      "f60cc1a688815d731f4600bbfc9fa36a0391ccd0590f44947426b959cc7bc263",
      "862cc8c7f0e2c7f6235b6a02d450eb59931a15e7921b0d2822062f8405f2c808",
      "700e2e1468608e33154ffc42be8d99830a3c026831585d86c66c464299c0a9d6",
      "6c192f4104f6749e71c56a147c0b352dbbb491d3a482fd6e066c722dd39846db",
      "65949642b3778ac7434c415d25a1b3c44ffd3b6400ad2ad7ad284db07d321b42",
      "82d3d297de7420c8f864a06423a36c899c8bd1412acca7dc8a72223ae41b17d4",
      "ac2ff668a9e0ce6b2acf7e69d24e5972392e4e0ee27852a9aea07d1f7391f5cd",
    ],
  },
  {
    prefix: "for-in-await-expr",
    features: ["top-level-await"],
    requiredBodyFragments: [
      "\nvar binding;\n",
      "for (var binding in",
      "for (let binding in",
    ],
    hashes: [
      "250baab9f7207028b05be5a5342a7e23b09c257962faebebcdc853e632e7eb97",
      "9befbb787dc0d62ee43f3e35b2d777119d21bd619e444c06769323dfcba7dd70",
      "abf9b523e120ec624b076780dfddf92f3f1c3c69bc6a13d296f7cd861fe7bbea",
      "52df7dd8abdec8423c075c2fdf15d35df02e6595ae5fe20cb0147942b214a21e",
      "d04952e5e9f1b8367d62b3c9d499e6c830fe53ca8cb67e9aa45c959969be36cd",
      "2d006c134e1d532c85be705ae45a710e0576ead067d1bd38a42cf6e407a17111",
      "530ac38d664f28e6199154808d94669194410bc3705b918498467b52be82ec15",
      "fe98c649b07d5b8a81ea6b60e50d937433651a63ef6b8d7ca1af865d9d5c9e1c",
      "411dc16722e7b3339cf6b828a4abd6e3fcf3a5d3ba53c3a31fbf7488d1ef4ece",
      "a5afd6aef6c78d1eb69e852fefdc1c96dbb0821f7fad41932daedfc964fb9a35",
      "fe5f00895514656e15ec9fb3e33be60ee13e2a96a82e5be40d343a389fe48b4b",
      "8eb38a7c1a049c6dd9bc30edf3a5bc7c723e256d46308511c4f43ca5ae19ad8c",
    ],
  },
  {
    prefix: "for-of-await-expr",
    features: ["top-level-await"],
    requiredBodyFragments: [
      "\nvar binding;\n",
      "for (var binding of",
      "for (let binding of",
    ],
    hashes: [
      "04707c36d3f3c4dcf878033ca4e5281ff338f967d18c514b2bef081084364c01",
      "906b591a8e1a47e35d52c9afc07cc3a038ce0482e33044526e322d3f8c33ffdf",
      "5cbf2dcbd006fe9187e472dace44fb664f403034d93cc8abff7c5fa336bd33f6",
      "bcecb50bc8d1ecd2a4414e4d616e8997d52e38c23ae29e3303a79a5c67f3e3f4",
      "401e4937764b6af9b3c0940ae3245cde6e13d171b63ea6b27c919cad52d29e4d",
      "c9bb3e11cd1819e32112f4d7d5ec5a298c1d63d88cc5a4e1e125e4730adcfc77",
      "71b6b2c82e558a9e42fdfea4dd2468e37543073390919d2684656dcb16623561",
      "e706fec3b7781151aeb3e43e260a6f70cb684010b30a87311bdab5bf7707249c",
      "a996572121fe7ebaf14b99aad1200d276537e016a9341bc2e222436c795897a2",
      "f6addefac0b8c2e21e080fd366ff8c2f564d2f4ee7385ef71849fa2a0c7df489",
      "ad6a2da7cdd254728448483c83a0453879fda5b747a7b9cf043b4de7284af9fb",
      "4b32cfa81f065e8857aea18f37af571df1add49448c1be42feb9ded143dcf851",
    ],
  },
];

const bytewise = (left, right) => Buffer.compare(Buffer.from(left), Buffer.from(right));
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
    // Test262 metadata is stored as a BTreeSet by the runner.
    flags: arrayField(text, "flags").sort(bytewise),
    features: arrayField(text, "features"),
    negativePhase: negative?.[1] ?? "",
    negativeType: negative?.[2] ?? "",
  };
}

assert.deepEqual(suffixes, [...suffixes].sort(bytewise), "suffixes must remain bytewise sorted");
assert.deepEqual(
  families.map(({ prefix }) => prefix),
  families.map(({ prefix }) => prefix).sort(bytewise),
  "families must remain bytewise sorted",
);
for (const family of families) {
  assert.equal(family.hashes.length, suffixes.length, `${family.prefix}: hash count changed`);
}

const roots = families.flatMap(({ prefix }) =>
  suffixes.map((suffix) => `${cohort}/${prefix}-${suffix}.js`),
);
const expectedSourceSha256 = new Map(
  families.flatMap(({ prefix, hashes }) =>
    suffixes.map((suffix, index) => [`${cohort}/${prefix}-${suffix}.js`, hashes[index]]),
  ),
);
assert.equal(roots.length, 48);
assert.equal(new Set(roots).size, roots.length);
assert.deepEqual(roots, [...roots].sort(bytewise), "roots must remain bytewise sorted");
assert.deepEqual([...expectedSourceSha256.keys()], roots);

// Construct the cohort only from the audited cross-product above, then use
// discovery as a canary so a new upstream case cannot enter silently.
const familyPattern = new RegExp(
  `^(?:${families.map(({ prefix }) => prefix).join("|")})-.+\\.js$`,
);
const discovered = readdirSync(join(suite, cohort), { withFileTypes: true })
  .filter((entry) => entry.isFile() && familyPattern.test(entry.name))
  .map((entry) => `${cohort}/${entry.name}`)
  .sort(bytewise);
assert.deepEqual(discovered, roots, "audited TLA loop family membership changed");

for (const family of families) {
  for (const suffix of suffixes) {
    const relativePath = `${cohort}/${family.prefix}-${suffix}.js`;
    const text = source(relativePath);
    const body = text.replace(frontmatter(text), "");
    assert.equal(
      sha256(text),
      expectedSourceSha256.get(relativePath),
      `${relativePath}: pinned source changed`,
    );
    assert.deepEqual(metadata(relativePath), {
      includes: [],
      flags: ["generated", "module"],
      features: family.features,
      negativePhase: "",
      negativeType: "",
    });
    for (const fragment of family.requiredBodyFragments) {
      assert(body.includes(fragment), `${relativePath}: missing loop binding shape ${fragment}`);
    }
    assert(!/^\s*(?:import|export)\b/m.test(body), `${relativePath}: static dependency added`);
    assert(!/\bimport\s*\(/.test(body), `${relativePath}: dynamic dependency added`);
  }
}

assert.equal(
  roots.filter((relativePath) => metadata(relativePath).features.includes("async-iteration"))
    .length,
  12,
);
assert.equal(roots.filter((relativePath) => metadata(relativePath).features.length === 1).length, 36);

const admissionRecords = roots.map((relativePath) => {
  const shape = metadata(relativePath);
  return admissionRecord({
    kind: "module",
    group: admissionGroup,
    path: relativePath,
    source_sha256: sha256(source(relativePath)),
    includes: shape.includes,
    flags: shape.flags,
    features: shape.features,
    negative_phase: shape.negativePhase,
    negative_type: shape.negativeType,
  });
});
assert.equal(admissionRecords.length, 48);

if (mode === "--admissions") {
  process.stdout.write(renderAdmissionRows(admissionRecords));
  process.exit(0);
}

function configSection(text, name) {
  const lines = text.split(/\r?\n/);
  const start = lines.indexOf(`[${name}]`);
  assert.notEqual(start, -1, `${quickjsConfig}: missing [${name}]`);
  const end = lines.findIndex((line, index) => index > start && /^\[.+\]$/.test(line));
  return lines
    .slice(start + 1, end === -1 ? undefined : end)
    .map((line) => line.replace(/#.*$/, "").trim())
    .filter(Boolean);
}

const quickjsConfigText = readFileSync(quickjsConfig, "utf8");
const configEntries = new Map(
  configSection(quickjsConfigText, "config").map((line) => {
    const separator = line.indexOf("=");
    assert.notEqual(separator, -1, `${quickjsConfig}: malformed config line ${line}`);
    return [line.slice(0, separator).trim(), line.slice(separator + 1).trim()];
  }),
);
assert.equal(configEntries.get("module"), "yes", "pinned QuickJS config skips modules");
const quickjsFeatures = new Map(
  configSection(quickjsConfigText, "features").map((line) => {
    const [name, value = "yes"] = line.split("=", 2);
    return [name.trim(), value.trim()];
  }),
);
for (const feature of ["top-level-await", "async-iteration"]) {
  assert.equal(quickjsFeatures.get(feature), "yes", `pinned QuickJS skips ${feature}`);
}

const quickjsRoot = dirname(quickjsConfig);
const suiteArguments = roots.map((relativePath) => {
  const argument = relative(quickjsRoot, join(suite, relativePath)).split(sep).join("/");
  assert(
    argument && !argument.startsWith("../"),
    `suite must be below the QuickJS Test262 source root: ${suite}`,
  );
  return argument;
});
const errorFileName = configEntries.get("errorfile");
assert(errorFileName, `${quickjsConfig}: missing errorfile`);
const errorFile = join(quickjsRoot, errorFileName);
assert(existsSync(errorFile), `missing pinned QuickJS error file: ${errorFile}`);
const knownErrorPaths = new Set(
  readFileSync(errorFile, "utf8")
    .split(/\r?\n/)
    .flatMap((line) => line.match(/^(.*?):\d+: /)?.[1] ?? []),
);
for (const argument of suiteArguments) {
  assert(!knownErrorPaths.has(argument), `${argument}: QuickJS pass masked by known-error ledger`);
}

const result = spawnSync(
  quickjsRunner,
  ["-v", "-T", "1", "-c", basename(quickjsConfig), "-f", ...suiteArguments],
  { cwd: quickjsRoot, encoding: "utf8" },
);
assert.equal(result.error, undefined, `QuickJS TLA loop oracle failed to start: ${result.error}`);
assert.equal(result.signal, null, "QuickJS TLA loop oracle terminated by signal");
assert.equal(
  result.status,
  0,
  `QuickJS TLA loop oracle failed:\n${result.stdout}${result.stderr}`,
);
const transcript = `${result.stdout}${result.stderr}`.replaceAll("\r\n", "\n");
assert(!/\bSKIPPED\b/.test(transcript), `QuickJS skipped an audited root:\n${transcript}`);
assert(!/^(?:.*Error|FAIL):/m.test(transcript), `QuickJS reported a failure:\n${transcript}`);

assertAdmissionGroup(checkedAdmissions, admissionGroup, admissionRecords);
console.log(
  "module-var-lexical-shadow-a current baseline authenticated: " +
    "roots=48 variants=48 dependencies=0 async_iteration=12 quickjs=48/48",
);
