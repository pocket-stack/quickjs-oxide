"""Project-authored scaling workloads; counts are fixed before timing begins.

Each generator returns JavaScript and an independently calculated stdout value.
These are diagnostic whole-process workloads, not upstream benchmark scores.
"""
from pathlib import Path

from run import digest

CASES = ("map-int", "map-string", "set", "map-churn", "set-churn",
         "set-intersection", "prop-write", "prop-delete", "array-truncate",
         "scope", "constants", "module", "module-imports", "long-key",
         "map-iterate-churn", "set-iterate-churn", "array-index", "array-holey",
         "typed-index", "arguments", "mapped-arguments", "regexp-groups")


BATCHED_CASES = ("map-int", "map-string", "set", "set-intersection", "prop-delete", "array-truncate",
                 "array-index", "typed-index", "arguments", "mapped-arguments", "regexp-groups")

def prepare(directory, case, size, operations):
    if case not in CASES or size < 1 or operations < 1:
        raise ValueError("known case and positive size/operations required")
    if case in BATCHED_CASES and operations % size:
        raise ValueError("size must divide operations to preserve fixed work")
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / "workload.mjs"
    files = []
    prefix = f"const size = {size}, operations = {operations};\nlet sum = 0;\n"
    if case in ("map-int", "map-string", "set"):
        key = '"key" + i' if case == "map-string" else "i"
        constructor = "Set" if case == "set" else "Map"
        insert = f"c.add({key})" if case == "set" else f"c.set({key}, i)"
        body = f"""for (let batch = 0; batch < operations / size; batch++) {{
  const c = new {constructor}();
  for (let i = 0; i < size; i++) {insert};
  for (let i = 0; i < size; i++) sum += c.has({key}) ? 1 : 0;
}}"""
        expected = operations
    elif case in ("map-churn", "set-churn"):
        # size controls history; live size stays one. Query work stays fixed.
        constructor = "Set" if case == "set-churn" else "Map"
        insert = "c.add(0)" if case == "set-churn" else "c.set(0, 0)"
        body = f"""const c = new {constructor}();
for (let i = 0; i < size; i++) {{ {insert}; c.delete(0); }}
{insert};
for (let i = 0; i < operations; i++) sum += c.has(0) ? 1 : 0;
sum += c.size;"""
        expected = operations + 1
    elif case in ("map-iterate-churn", "set-iterate-churn"):
        constructor = "Set" if case.startswith("set") else "Map"
        insert = "c.add(i)" if constructor == "Set" else "c.set(i, i)"
        first = "c.add(-1)" if constructor == "Set" else "c.set(-1, -1)"
        last = "c.add(1)" if constructor == "Set" else "c.set(1, 1)"
        body = f"""const c = new {constructor}();
{first};
const paused = c.keys();
paused.next();
c.delete(-1);
for (let i = 0; i < size; i++) {{ {insert}; c.delete(i); }}
{last};
sum = paused.next().value;
for (let i = 0; i < operations; i++) sum += c.keys().next().value;
if (!paused.next().done) throw Error('paused cursor did not finish');"""
        expected = operations + 1
    elif case == "set-intersection":
        body = """for (let batch = 0; batch < operations / size; batch++) {
  const a = new Set(), b = new Set();
  for (let i = 0; i < size; i++) { a.add(i); b.add(i); }
  sum += a.intersection(b).size;
}"""
        expected = operations
    elif case == "prop-write":
        body = """const o = {};
for (let i = 0; i < size; i++) o['p' + i] = i;
for (let i = 0; i < operations; i++) o.p0 = i;
sum = o.p0 + Object.keys(o).length;"""
        expected = operations - 1 + size
    elif case == "prop-delete":
        body = """for (let batch = 0; batch < operations / size; batch++) {
  const o = {};
  for (let i = 0; i < size; i++) o['p' + i] = i;
  for (let i = 0; i < size; i++) sum += delete o['p' + i] ? 1 : 0;
  if (Object.keys(o).length !== 0) throw Error('delete failed');
}"""
        expected = operations
    elif case == "array-truncate":
        body = """for (let batch = 0; batch < operations / size; batch++) {
  const a = [];
  for (let i = 0; i < size; i++) a.push(i);
  delete a[0];
  a.length = 0;
  if (Object.keys(a).length !== 0) throw Error('truncate failed');
  sum += size;
}"""
        expected = operations
    elif case in ("array-index", "typed-index"):
        constructor = "new Array(size).fill(0)" if case == "array-index" else "new Uint32Array(size)"
        body = f"""const a = {constructor};
for (let i = 0; i < operations; i++) {{ const index = i % size; a[index] = index; sum += a[index]; }}"""
        expected = operations * (size - 1) // 2
    elif case == "array-holey":
        body = """const a = [];
for (let i = 0; i < size; i++) a.push(i);
const index = Math.floor(size / 2);
for (let i = 0; i < operations; i++) { delete a[index]; a[index] = i; sum += a[index]; }
if (a.length !== size) throw Error('length changed');"""
        expected = operations * (operations - 1) // 2
    elif case in ("arguments", "mapped-arguments"):
        function = ('function f() { "use strict"; return arguments.length; }' if case == "arguments"
                    else 'const f = Function("first", "first = 1; if (arguments[0] !== 1) throw Error(\'alias lost\'); return arguments.length;");')
        body = f"""{function}
const args = new Array(size).fill(0);
for (let i = 0; i < operations / size; i++) sum += f.apply(null, args);"""
        expected = operations
    elif case == "regexp-groups":
        if size >= 255:
            raise ValueError("regexp-groups size must fit the pinned capture limit (<255)")
        body = """let pattern = '';
for (let i = 0; i < size; i++) pattern += '(?<g' + i + '>a)';
const re = new RegExp(pattern, 'd'), input = 'a'.repeat(size);
for (let i = 0; i < operations / size; i++) {
  const result = re.exec(input);
  if (result.groups.g0 !== 'a' || result.indices.groups.g0 !== result.indices[1]) throw Error('capture mismatch');
  sum += Object.keys(result.groups).length;
}"""
        expected = operations
    elif case == "scope":
        declarations = "".join(f"let v{i} = {i};\n" for i in range(size))
        references = "+".join(f"v{i}" for i in range(size))
        body = f"function f() {{\n{declarations}\nreturn {references};\n}}\nsum = f();"
        expected = size * (size - 1) // 2
    elif case == "constants":
        # typeof unresolved globals exercises compiler name constants without throwing.
        terms = "\n".join(f"sum += typeof global_{i} === 'undefined' ? 1 : 0;" for i in range(size))
        body = f"function f() {{\n{terms}\n}}\nf();"
        expected = size
    elif case in ("module", "module-imports"):
        dependency = directory / "exports.mjs"
        dependency.write_text("".join(f"export const v{i} = {i};\n" for i in range(size)))
        files.append(dependency)
        entry = "exports.mjs"
        if case == "module-imports":
            entry = "imports.mjs"
            names = ", ".join(f"v{i}" for i in range(size))
            reexport = directory / entry
            reexport.write_text(f"import {{ {names} }} from './exports.mjs';\nexport {{ {names} }};\n")
            files.append(reexport)
        body = f"import * as ns from './{entry}';\nfor (const key of Object.keys(ns)) sum += ns[key];"
        expected = size * (size - 1) // 2
    else:  # long-key: repeated content lookup; the two strings are built separately.
        body = """const a = 'x'.repeat(size) + '!', b = 'x'.repeat(size) + '!';
const c = new Map([[a, 1]]);
for (let i = 0; i < operations; i++) sum += c.get(b);
"""
        expected = operations
    path.write_text(prefix + body + "\nconsole.log(String(sum));\n")
    files.append(path)
    return {"case": case, "size": size, "operations": operations,
            "path": str(path.resolve()), "expected": f"{expected}\n",
            "files": {f.name: digest(f) for f in files}}
