"""Generate fixed ordinary-property diagnostics and authenticate outputs with Node.

Timing belongs to fixed.py. These workloads include process startup, compilation
and setup; depth setup-only controls are separate whole-process measurements.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def programs():
    for depth in (0, 1, 8, 64, 256):
        for operation, count in (("write", 200000), ("setup", 0)):
            yield f"own-{operation}-depth-{depth}", depth, (
                f"let p=null; for(let i=0;i<{depth};i++) p=Object.create(p);\n"
                "let o=Object.create(p);o.x=0;\n"
                f"for(let i=0;i<{count};i++) o.x=i;\nconsole.log(o.x);\n"
            )

    def loop(case, setup, body, result, size=0, count=100000):
        return case, size, (
            f"{setup}\nfor(let i=0;i<{count};i++){{{body}}}\n"
            f"console.log({result});\n"
        )

    for width in (4, 32, 256, 2048):
        for mode in ("shape", "dictionary"):
            setup = f'let o={{}}; for(let k=0;k<{width};k++)o["p"+k]=0;'
            if mode == "dictionary":
                setup += "delete o.p1;"
            yield loop(f"write-width-{width}-{mode}", setup, "o.p0=i", "o.p0", width)
    values = (
        ("int", "[1,2]", "1"),
        ("float", "[1.5,2.5]", "1.5"),
        ("string", '["alpha","beta"]', '"alpha"'),
        ("symbol", '[Symbol("a"),Symbol("b")]', 'Symbol("a")'),
        ("object", "[{},{}]", "{}"),
    )
    for kind, alternating, _ in values:
        yield loop("write-value-" + kind, "let v=" + alternating + ";let o={x:v[0]};",
                   "o.x=v[i&1]", "o.x===v[1]")
    for args in (
        ("get-data", "let o={x:3};let s=0;", "s+=o.x", "s"),
        ("get-inherited", "let o=Object.create({x:3});let s=0;", "s+=o.x", "s"),
        ("get-accessor", "let o={get x(){return 3}};let s=0;", "s+=o.x", "s"),
        ("set-accessor", "let s=0;let o={set x(v){s=v}};", "o.x=i", "s"),
        ("set-receiver", "let p={x:0},o={x:0};", 'Reflect.set(p,"x",i,o)', "o.x"),
        ("define-value", "let o={x:0};let d={value:0};", 'd.value=i;Object.defineProperty(o,"x",d)', "o.x"),
        ("has-own", "let o={x:1};let s=0;", 'if(Object.hasOwn(o,"x"))s++', "s"),
        ("enumerable", "let o={x:1};let s=0;", 'if(o.propertyIsEnumerable("x"))s++', "s"),
        ("proxy-set", "let target={x:0};let p=new Proxy(target,{});", "p.x=i", "target.x", 0, 20000),
        ("missing-get", "let o=Object.create({});let s=0;", "if(o.x===undefined)s++", "s"),
    ):
        yield loop(*args)
    for kind, _, value in values:
        yield "write-same-value-" + kind, 100000, (
            f"let v={value};let o={{x:v}};for(let i=0;i<100000;i++){{o.x=v}} console.log(o.x===v);\n"
        )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new workload directory")
    parser.add_argument("--oracle", default="node", help="Node executable for output authentication")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = {key: value for key, value in os.environ.items() if key != "FORCE_COLOR"}
    env["NO_COLOR"] = "1"
    rows = []
    for case, size, source in programs():
        path = output / (case + ".js")
        path.write_text(source)
        result = subprocess.run([args.oracle, str(path)], env=env, capture_output=True,
                                check=True, timeout=30)
        if result.stderr:
            raise RuntimeError(f"unexpected oracle stderr for {case}: {result.stderr!r}")
        rows.append(dict(case=case, size=size, path=str(path),
                         sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                         expected=result.stdout.decode()))
    manifest = output / "manifest.json"
    manifest.write_text(json.dumps({"metadata": {"workloads": {"workloads": rows}}}, indent=2) + "\n")
    print(f"{len(rows)} authenticated fixed workloads: {manifest}")


if __name__ == "__main__":
    main()
