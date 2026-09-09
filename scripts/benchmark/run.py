#!/usr/bin/env python3
"""Run external JS workloads without vendoring them or conflating failed runs with scores."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import signal
import statistics
import subprocess
import time
import threading
import tempfile

ROOT = Path(__file__).resolve().parents[2]
LOAD = re.compile(r"load\('([^']+)'\);")
MICROBENCH_CLOCK_PREFIX = (
    "// Identical host clock adaptation; benchmark bodies below are unchanged.\n"
    "var performance = undefined; var os = undefined;\n"
    'console.log("__oxide_clock__:" + (typeof performance !== "undefined" ? '
    '"performance.now" : typeof os !== "undefined" ? "os.now" : "Date.now"));\n'
)
NUMBER = r"(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?"


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def command_output(command, cwd=None):
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True, timeout=15)
    return {"command": list(map(str, command)), "exit_code": result.returncode,
            "stdout": result.stdout.strip(), "stderr": result.stderr.strip()}


def git_metadata(path):
    return {"commit": command_output(["git", "rev-parse", "HEAD"], path),
            "status": command_output(["git", "status", "--porcelain"], path)}


def binary_metadata(path):
    metadata = {"path": str(path), "sha256": digest(path),
                "version": command_output([str(path), "--version"])}
    receipt = path.with_suffix(".build.json")
    if receipt.exists():
        build = json.loads(receipt.read_text())
        if build.get("binary_sha256") != metadata["sha256"]:
            raise ValueError(f"stale build receipt: {receipt}")
        metadata["build"] = build
    else:
        metadata["build"] = None
    return metadata


def machine_metadata():
    cpu = None
    if Path("/proc/cpuinfo").exists():
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    return {"platform": platform.platform(), "machine": platform.machine(), "cpu": cpu,
            "logical_cpus": os.cpu_count(), "python": platform.python_version(),
            "runner_sha256": digest(__file__), "repository": git_metadata(ROOT)}


def prepare_v8(source, cases):
    """Match scripts/build.ts: inline load() for all, base+case+runner for each suite."""
    source = source.resolve()
    if source.is_relative_to(ROOT):
        raise ValueError("js-engine-benchmark must be checked out outside quickjs-oxide")
    code_root = source / "v8-v7"
    run = (code_root / "run.js").read_text()
    loads = list(LOAD.finditer(run))
    if not loads or loads[0][1] != "base.js":
        raise ValueError("unrecognized external run.js: expected base.js as first load")
    contents = {}
    for match in loads:
        name = match[1]
        path = (code_root / name).resolve()
        if not path.is_relative_to(code_root) or path.suffix != ".js":
            raise ValueError("unexpected workload path in external run.js")
        contents[name] = path.read_text()
    available = [Path(name).stem for name in contents if name != "base.js"]
    selected = cases or available
    if len(set(selected)) != len(selected) or set(selected) - set(available + ["all"]):
        raise ValueError(f"choose unique cases from {available + ['all']}")
    # Generated sources always remain in the external checkout, never this repo.
    dist = (source / "dist" / "quickjs-oxide").resolve()
    if not dist.is_relative_to(source) or dist.is_relative_to(ROOT):
        raise ValueError("generated bundles must stay in the external checkout")
    dist.mkdir(parents=True, exist_ok=True)
    combined = LOAD.sub(lambda m: contents[m[1]], run)
    (dist / "run.js").write_text(combined)
    runner = run[loads[-1].end():]
    workloads = []
    for case in selected:
        code = combined if case == "all" else "\n".join([contents["base.js"], contents[case + ".js"], runner])
        path = dist / ("run.js" if case == "all" else case + ".js")
        path.write_text(code)
        names = re.findall(r"new BenchmarkSuite\(\s*['\"]([^'\"]+)['\"]", code)
        if not names or len(set(names)) != len(names):
            raise ValueError(f"cannot identify unique expected suite scores for {case}")
        workloads.append({"case": case, "path": str(path), "args": [], "sha256": digest(path), "expected": names})
    return workloads, {"repository": git_metadata(source), "source": str(source),
                       "files": {name: digest(code_root / name) for name in contents},
                       "run_js_sha256": digest(code_root / "run.js"),
                       "adaptation": "only upstream scripts/build.ts load inlining and standalone concatenation; no benchmark-body or timing changes",
                       "timer": "external harness Date millisecond clock", "metric": "suite score; higher is better"}


def prepare_microbench(source, cases):
    source = source.resolve()
    if not source.is_file():
        raise ValueError("--source must point to the pinned QuickJS tests/microbench.js")
    selected = cases or ["empty_loop", "prop_read", "array_read", "func_call", "int_arith"]
    if len(set(selected)) != len(selected) or any(not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", c) for c in selected):
        raise ValueError("microbench cases must be unique function-name prefixes")
    original = source.read_bytes()
    text = original.decode("utf-8")
    # Prefix matching is the unmodified upstream harness behavior. Preserve it.
    test_list = re.search(r"var test_list = \[(.*?)\];", text, re.S)
    if test_list is None:
        raise ValueError("unrecognized microbench test list")
    names = re.findall(r"\b[A-Za-z_]\w*\b", test_list[1])
    names.extend(re.findall(r"test_list.push\((\w+)\)", text))
    prepared_dir = Path(tempfile.mkdtemp(prefix="quickjs-oxide-microbench-"))
    prepared = prepared_dir / "microbench.js"
    prepared.write_bytes(MICROBENCH_CLOCK_PREFIX.encode() + original)
    workloads = []
    for case in selected:
        expected = [name for name in names if name.startswith(case)]
        if not expected:
            raise ValueError(f"unknown microbench prefix: {case}")
        workloads.append({"case": case, "path": str(prepared), "args": [case],
                          "sha256": digest(prepared), "expected": expected})
    return workloads, {"source": str(source), "sha256": digest(source), "prepared_source": str(prepared),
                       "prepared_sha256": digest(prepared), "adaptation": "identical prefix disables optional performance/os clocks on both engines; original body bytes unchanged",
                       "adaptation_prefix": MICROBENCH_CLOCK_PREFIX,
                       "adaptation_sha256": hashlib.sha256(MICROBENCH_CLOCK_PREFIX.encode()).hexdigest(),
                       "timer": "verified Date.now fallback on every run (millisecond clock, rounded to next tick)",
                       "metric": "upstream minimum ns/op; lower is better; not a distribution of individual operations"}


def parse_v8(text, expected):
    scores = {}
    for line in text.splitlines():
        match = re.fullmatch(rf"([^:]+):\s*({NUMBER})", line.strip())
        if match and (match[1] in expected or match[1] == "Score"):
            if match[1] in scores:
                raise ValueError("duplicate score line")
            scores[match[1]] = float(match[2])
    if set(scores) != set(expected + ["Score"]) or any(not math.isfinite(v) or v <= 0 for v in scores.values()):
        raise ValueError("missing or invalid suite/aggregate score (the harness may swallow JS errors)")
    return scores


def parse_microbench(text, expected):
    clocks = [line for line in text.splitlines() if line.startswith("__oxide_clock__:")]
    if clocks != ["__oxide_clock__:Date.now"]:
        raise ValueError("missing or mismatched clock proof; microbench requires identical Date.now fallback")
    values = {}
    for line in text.splitlines():
        match = re.fullmatch(rf"\s*(\w+)\s+(\d+)\s+({NUMBER})\s*", line)
        if match and match[1] in expected:
            if match[1] in values:
                raise ValueError("duplicate benchmark row")
            value = float(match[3])
            if not math.isfinite(value) or value <= 0 or int(match[2]) <= 0:
                raise ValueError("invalid benchmark time/iteration count")
            values[match[1]] = {"ns_per_op": value, "n_argument": int(match[2])}
    if set(values) != set(expected):
        raise ValueError("missing benchmark row; failed/unsupported cases are not zero ns")
    return values


def run_sample(command, cwd, prefix, timeout):
    # A watchdog allows blocking waitpid instead of Popen.wait(timeout)'s
    # exponentially backed-off polling, which quantizes short wall timings.
    expired = threading.Event()
    with prefix.with_suffix(".stdout").open("wb") as stdout, prefix.with_suffix(".stderr").open("wb") as stderr:
        started = time.perf_counter_ns()
        proc = subprocess.Popen(command, cwd=cwd, stdout=stdout, stderr=stderr, start_new_session=True)

        def kill_group():
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

        def expire():
            if proc.poll() is None:
                expired.set()
                kill_group()

        watchdog = threading.Timer(timeout, expire)
        watchdog.daemon = True
        watchdog.start()
        try:
            code = proc.wait()
            elapsed = time.perf_counter_ns() - started
        except BaseException:
            kill_group()
            proc.wait()
            raise
        finally:
            watchdog.cancel()
            watchdog.join()
    return {"command": command, "exit_code": code, "timed_out": expired.is_set(),
            "process_wall_ns": elapsed,
            "stdout": str(prefix.with_suffix(".stdout")), "stderr": str(prefix.with_suffix(".stderr"))}


def summarize(samples, suite):
    groups = {}
    for sample in samples:
        key = (sample["case"], sample["engine"])
        group = groups.setdefault(key, {"case": key[0], "engine": key[1], "samples": 0, "successful": 0, "failures": [], "values": {}})
        group["samples"] += 1
        if sample["status"] != "ok":
            group["failures"].append(sample["status"])
            continue
        group["successful"] += 1
        for name, value in sample["measurements"].items():
            value = value if suite == "v8-v7" else value["ns_per_op"]
            group["values"].setdefault(name, []).append(value)
    results = []
    for group in groups.values():
        group["eligible_for_comparison"] = group["successful"] == group["samples"]
        group["metrics"] = {name: {"raw": values, "minimum": min(values), "median": statistics.median(values),
                                   "maximum": max(values), "stdev": statistics.stdev(values) if len(values) > 1 else None}
                            for name, values in group.pop("values").items()}
        results.append(group)
    return results


def write_report(output, results, suite, names):
    lines = [f"# {suite} benchmark results", "", "Sequential runs with alternating engine order. Raw samples and metadata are in results.json and samples.jsonl.",
             "", "Scores are higher-is-better; ns/op is lower-is-better. Process wall time includes startup, parsing, execution and teardown and is not the operation metric.",
             "", "| Case | Engine | Successful runs | Metric | Median | Min–max |", "| --- | --- | ---: | --- | ---: | --- |"]
    for group in results:
        metric_names = ["Score"] if suite == "v8-v7" and "Score" in group["metrics"] else list(group["metrics"])
        if not metric_names:
            lines.append(f"| {group['case']} | {group['engine']} | {group['successful']}/{group['samples']} | unavailable ({', '.join(group['failures'])}) | — | — |")
        for metric_name in metric_names:
            metric = group["metrics"][metric_name]
            lines.append(f"| {group['case']} | {group['engine']} | {group['successful']}/{group['samples']} | {metric_name} | {metric['median']:.4g} | {metric['minimum']:.4g}–{metric['maximum']:.4g} |")
    if len(names) == 2:
        lines.extend(["", f"Comparison: {names[0]} relative to {names[1]} (speed ratio; >1 means the first engine is faster). Only cases with all requested runs successful on both engines qualify.", ""])
        by_key = {(g["case"], g["engine"]): g for g in results}
        for case in dict.fromkeys(g["case"] for g in results):
            first, second = [by_key.get((case, name)) for name in names]
            if not first or not second or not all(g["eligible_for_comparison"] for g in [first, second]):
                continue
            for metric_name in first["metrics"].keys() & second["metrics"].keys():
                if suite == "v8-v7" and metric_name != "Score":
                    continue
                a, b = [g["metrics"][metric_name]["median"] for g in [first, second]]
                ratio = a / b if suite == "v8-v7" else b / a
                lines.append(f"- {case}/{metric_name}: {ratio:.4g}×")
    lines.extend(["", "Failed, incomplete and timed-out cases remain visible and are excluded from ratios. This report does not infer a whole-engine score from a subset.", ""])
    (output / "report.md").write_text("\n".join(lines))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=["v8-v7", "microbench"], required=True)
    parser.add_argument("--source", type=Path, required=True, help="external checkout, or pinned microbench.js")
    parser.add_argument("--engine", action="append", required=True, help="NAME=/absolute/path/to/qjs (repeat for comparison)")
    parser.add_argument("--case", action="append", dest="cases")
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=120, help="seconds per case and process")
    parser.add_argument("--output", type=Path, required=True, help="new result directory")
    args = parser.parse_args()
    if args.repeat < 1 or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("repeat and timeout must be positive and finite")
    engines = {}
    for item in args.engine:
        name, sep, path = item.partition("=")
        if not sep or not re.fullmatch(r"[A-Za-z0-9_-]+", name) or name in engines:
            parser.error("use unique NAME=/path/to/binary engine specifications")
        binary = Path(path).resolve()
        if not binary.is_file() or not os.access(binary, os.X_OK):
            parser.error(f"engine is not an executable file: {binary}")
        engines[name] = binary
    prepare = prepare_v8 if args.suite == "v8-v7" else prepare_microbench
    try:
        workloads, source = prepare(args.source, args.cases)
    except (ValueError, OSError) as error:
        parser.error(str(error))
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    (output / "raw").mkdir()
    metadata = {"schema": "oxide-benchmark-v1", "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "suite": args.suite, "machine": machine_metadata(), "source": source, "workloads": workloads,
                "repeat": args.repeat, "timeout_seconds": args.timeout, "order": "serial, alternating per repetition",
                "instrumentation": "off (no -d/-T)", "engines": {name: binary_metadata(path) for name, path in engines.items()}}
    (output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    samples = []
    with (output / "samples.jsonl").open("w") as journal:
        for workload in workloads:
            for iteration in range(args.repeat):
                order = list(engines) if iteration % 2 == 0 else list(reversed(engines))
                for name in order:
                    prefix = output / "raw" / f"{workload['case']}-{name}-{iteration}"
                    command = [str(engines[name]), workload["path"], *workload["args"]]
                    sample = run_sample(command, output, prefix, args.timeout)
                    sample.update(case=workload["case"], engine=name, repetition=iteration, workload_sha256=workload["sha256"])
                    sample["status"] = "timeout" if sample["timed_out"] else "failed" if sample["exit_code"] else "ok"
                    if sample["status"] == "ok":
                        try:
                            parse = parse_v8 if args.suite == "v8-v7" else parse_microbench
                            sample["measurements"] = parse(Path(sample["stdout"]).read_text(errors="replace"), workload["expected"])
                        except ValueError as error:
                            sample.update(status="incomplete-or-unsupported", reason=str(error))
                    samples.append(sample)
                    journal.write(json.dumps(sample) + "\n")
                    journal.flush()
                    print(f"{workload['case']} {name} #{iteration + 1}: {sample['status']}", flush=True)
    results = summarize(samples, args.suite)
    (output / "results.json").write_text(json.dumps({**metadata, "samples": samples, "summary": results}, indent=2) + "\n")
    write_report(output, results, args.suite, list(engines))
    print(output / "report.md")
    return 0 if all(s["status"] == "ok" for s in samples) else 1


if __name__ == "__main__":
    raise SystemExit(main())
