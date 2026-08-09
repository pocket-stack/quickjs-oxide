#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { lstatSync, readdirSync, readFileSync, realpathSync } from "node:fs";
import { isAbsolute, relative, resolve, sep } from "node:path";

const DOMAIN = Buffer.from("quickjs-oxide-test262-engine-semantics-v1\0", "utf8");

function die(message) {
  process.stderr.write(`test262-engine-fingerprint: ${message}\n`);
  process.exit(2);
}

function usage() {
  process.stderr.write(
    "usage: test262-engine-fingerprint.mjs --root DIR " +
      "(--worktree | --commit SHA) --files CSV --trees CSV\n",
  );
  process.exit(2);
}

function takeArguments(values) {
  const options = { root: undefined, source: undefined, files: undefined, trees: undefined };
  for (let index = 0; index < values.length; index += 1) {
    const argument = values[index];
    const take = () => {
      index += 1;
      if (index >= values.length) usage();
      return values[index];
    };
    if (argument === "--root") options.root = take();
    else if (argument === "--worktree") {
      if (options.source !== undefined) usage();
      options.source = { kind: "worktree" };
    } else if (argument === "--commit") {
      if (options.source !== undefined) usage();
      options.source = { kind: "commit", value: take() };
    } else if (argument === "--files") options.files = take();
    else if (argument === "--trees") options.trees = take();
    else usage();
  }
  if (
    options.root === undefined ||
    options.source === undefined ||
    options.files === undefined ||
    options.trees === undefined
  ) {
    usage();
  }
  return options;
}

function parsePathList(value, label) {
  const paths = value.split(",");
  if (paths.length === 0 || paths.some((path) => path.length === 0)) {
    die(`${label} must be a non-empty comma-separated path list`);
  }
  for (const path of paths) {
    if (
      isAbsolute(path) ||
      path.includes("\\") ||
      path.split("/").some((component) => component === "" || component === "." || component === "..")
    ) {
      die(`unsafe repository path in ${label}: ${path}`);
    }
  }
  const sorted = [...paths].sort((left, right) =>
    Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8")),
  );
  if (paths.some((path, index) => path !== sorted[index])) {
    die(`${label} must be bytewise sorted`);
  }
  if (new Set(paths).size !== paths.length) die(`${label} contains duplicate paths`);
  return paths;
}

function repositoryPath(root, path) {
  const absolute = resolve(root, ...path.split("/"));
  const inside = relative(root, absolute);
  if (inside === "" || inside === ".." || inside.startsWith(`..${sep}`) || isAbsolute(inside)) {
    die(`repository path escaped root: ${path}`);
  }
  return absolute;
}

function collectWorktree(root, files, trees) {
  const entries = [];
  const addFile = (path) => {
    const absolute = repositoryPath(root, path);
    const stat = lstatSync(absolute, { throwIfNoEntry: false });
    if (stat === undefined || !stat.isFile() || stat.isSymbolicLink()) {
      die(`input is not a regular non-symlink file: ${path}`);
    }
    entries.push([path, readFileSync(absolute)]);
  };
  const walk = (path) => {
    const absolute = repositoryPath(root, path);
    const stat = lstatSync(absolute, { throwIfNoEntry: false });
    if (stat === undefined || !stat.isDirectory() || stat.isSymbolicLink()) {
      die(`input tree is not a real directory: ${path}`);
    }
    const children = readdirSync(absolute, { withFileTypes: true }).sort((left, right) =>
      Buffer.compare(Buffer.from(left.name, "utf8"), Buffer.from(right.name, "utf8")),
    );
    for (const child of children) {
      const childPath = `${path}/${child.name}`;
      if (child.isSymbolicLink()) die(`symbolic link is forbidden in input tree: ${childPath}`);
      if (child.isDirectory()) walk(childPath);
      else if (child.isFile()) addFile(childPath);
      else die(`non-regular entry is forbidden in input tree: ${childPath}`);
    }
  };
  for (const path of files) addFile(path);
  for (const path of trees) walk(path);
  return entries;
}

function git(root, gitArguments, encoding = undefined) {
  try {
    return execFileSync("git", ["-C", root, ...gitArguments], {
      encoding,
      maxBuffer: 256 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const detail = error.stderr?.toString("utf8").trim() || error.message;
    die(`git ${gitArguments[0]} failed: ${detail}`);
  }
}

function gitBlobs(root, commit, paths) {
  for (const path of paths) {
    if (path.includes("\n") || path.includes("\r") || path.includes("\0")) {
      die(`commit input path cannot be represented in a batch request: ${JSON.stringify(path)}`);
    }
  }
  const input = `${paths.map((path) => `${commit}:${path}`).join("\n")}\n`;
  let output;
  try {
    output = execFileSync("git", ["-C", root, "cat-file", "--batch"], {
      input,
      maxBuffer: 256 * 1024 * 1024,
      stdio: ["pipe", "pipe", "pipe"],
    });
  } catch (error) {
    const detail = error.stderr?.toString("utf8").trim() || error.message;
    die(`git cat-file failed: ${detail}`);
  }
  const blobs = [];
  let offset = 0;
  for (const path of paths) {
    const newline = output.indexOf(0x0a, offset);
    if (newline === -1) die(`truncated git cat-file header for: ${path}`);
    const header = output.subarray(offset, newline).toString("utf8");
    const match = /^[0-9a-f]{40,64} blob ([0-9]+)$/.exec(header);
    if (match === null) die(`commit input is missing or is not a regular blob: ${path}`);
    const size = Number(match[1]);
    if (!Number.isSafeInteger(size)) die(`commit input is too large: ${path}`);
    const start = newline + 1;
    const end = start + size;
    if (end >= output.length || output[end] !== 0x0a) {
      die(`truncated git cat-file contents for: ${path}`);
    }
    blobs.push([path, output.subarray(start, end)]);
    offset = end + 1;
  }
  if (offset !== output.length) die("git cat-file returned trailing data");
  return blobs;
}

function collectCommit(root, commit, files, trees) {
  if (!/^[0-9a-f]{40}$/.test(commit)) die(`commit must be a full lowercase SHA-1: ${commit}`);
  const resolved = git(root, ["rev-parse", "--verify", `${commit}^{commit}`], "utf8").trim();
  if (resolved !== commit) die(`commit did not resolve exactly: ${commit}`);
  const paths = [...files];
  for (const tree of trees) {
    const output = git(root, ["ls-tree", "-r", "-z", "--name-only", commit, "--", tree]);
    const treePaths = [];
    let offset = 0;
    while (offset < output.length) {
      const nul = output.indexOf(0, offset);
      if (nul === -1) die(`git returned an unterminated path for input tree: ${tree}`);
      const bytes = output.subarray(offset, nul);
      const path = bytes.toString("utf8");
      if (!Buffer.from(path, "utf8").equals(bytes)) {
        die(`git returned a non-UTF-8 path for input tree: ${tree}`);
      }
      if (path.length !== 0) treePaths.push(path);
      offset = nul + 1;
    }
    if (treePaths.length === 0) die(`commit input tree is empty or missing: ${tree}`);
    for (const path of treePaths) {
      if (path !== tree && !path.startsWith(`${tree}/`)) {
        die(`git returned a path outside input tree ${tree}: ${path}`);
      }
      paths.push(path);
    }
  }
  return gitBlobs(root, commit, paths);
}

function encodeLength(value) {
  const output = Buffer.alloc(8);
  output.writeBigUInt64BE(BigInt(value));
  return output;
}

function fingerprint(entries) {
  entries.sort(([left], [right]) =>
    Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8")),
  );
  for (let index = 1; index < entries.length; index += 1) {
    if (entries[index - 1][0] === entries[index][0]) {
      die(`input path is covered more than once: ${entries[index][0]}`);
    }
  }
  const hash = createHash("sha256");
  hash.update(DOMAIN);
  for (const [path, contents] of entries) {
    const pathBytes = Buffer.from(path, "utf8");
    hash.update(encodeLength(pathBytes.length));
    hash.update(pathBytes);
    hash.update(encodeLength(contents.length));
    hash.update(contents);
  }
  return hash.digest("hex");
}

const options = takeArguments(process.argv.slice(2));
const root = realpathSync(options.root);
const stat = lstatSync(root);
if (!stat.isDirectory() || stat.isSymbolicLink()) die(`root is not a real directory: ${root}`);
const files = parsePathList(options.files, "--files");
const trees = parsePathList(options.trees, "--trees");
const entries =
  options.source.kind === "worktree"
    ? collectWorktree(root, files, trees)
    : collectCommit(root, options.source.value, files, trees);
process.stdout.write(`${fingerprint(entries)}\n`);
