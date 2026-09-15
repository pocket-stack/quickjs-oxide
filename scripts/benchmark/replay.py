"""Replay frozen compile-only or original V8 inputs; never regenerate workload bytes."""
import argparse
import json
import re
import statistics
from pathlib import Path

from run import binary_metadata, digest, machine_metadata, parse_v8, run_sample


def load_workloads(receipt, directory, mode, cases=None):
    workloads = json.loads(Path(receipt).read_text())["workloads"]
    selected = []
    names = set()
    for original in workloads:
        name = original["case"]
        if not re.fullmatch(r"[a-zA-Z0-9_-]+", name) or name in names:
            raise ValueError("workload names must be unique safe identifiers")
        names.add(name)
        # Original entries carry score names, fixed work carries expected text.
        if mode == "original" and not isinstance(original["expected"], list):
            continue
        if cases and name not in cases:
            continue
        item = dict(original)
        path = Path(directory) / Path(item["path"]).name if directory else Path(item["path"])
        if digest(path) != item["sha256"]:
            raise ValueError(f"workload bytes changed: {name}")
        item["path"] = str(path.resolve())
        selected.append(item)
    if not selected or (cases and (len(cases) != len(set(cases)) or set(cases) != {w["case"] for w in selected})):
        raise ValueError("requested cases must be unique and present in the selected matrix")
    return selected


def admit(sample, workload, mode):
    if sample["timed_out"]:
        return "timeout", {}
    if sample["exit_code"]:
        return "failed", {}
    if Path(sample["stderr"]).read_bytes():
        return "unexpected-stderr", {}
    text = Path(sample["stdout"]).read_text()
    if mode == "compile":
        match = re.fullmatch(r"compile_ns:(\d+)\n", text)
        return ("ok", {"compile_ns": int(match[1])}) if match else ("invalid-output", {})
    try:
        return "ok", parse_v8(text, workload["expected"])
    except ValueError:
        return "invalid-output", {}


def summarize(samples, names):
    groups = {}
    for sample in samples:
        group = groups.setdefault((sample["case"], sample["engine"]), [])
        group.append(sample)
    rows = []
    by_key = {}
    for (case, engine), group in groups.items():
        successful = [s for s in group if s["status"] == "ok"]
        metrics = {}
        for name in {key for sample in successful for key in sample["measurements"]}:
            values = [s["measurements"][name] for s in successful]
            metrics[name] = dict(minimum=min(values), median=statistics.median(values), maximum=max(values), raw=values)
        row = dict(case=case, engine=engine, samples=len(group), successful=len(successful),
                   eligible=len(successful) == len(group), metrics=metrics,
                   failures=[s["status"] for s in group if s["status"] != "ok"])
        rows.append(row)
        by_key[case, engine] = row
    ratios = []
    for case in dict.fromkeys(s["case"] for s in samples):
        for after in names[1:]:
            for before in names[:names.index(after)]:
                a, b = by_key[case, after], by_key[case, before]
                if not (a["eligible"] and b["eligible"]):
                    continue
                for metric in a["metrics"].keys() & b["metrics"].keys():
                    denominator = b["metrics"][metric]["median"]
                    if denominator:
                        ratios.append(dict(case=case, after=after, before=before, metric=metric,
                                           ratio=a["metrics"][metric]["median"] / denominator))
    return rows, ratios


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=["compile", "original"], required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--workload-dir", type=Path)
    parser.add_argument("--engine", action="append", required=True)
    parser.add_argument("--case", action="append")
    parser.add_argument("--repeat", type=int)
    parser.add_argument("--timeout", type=float, default=1800)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    repeat = args.repeat if args.repeat is not None else (10 if args.mode == "compile" else 5)
    if repeat < 1 or args.timeout <= 0:
        parser.error("repeat and timeout must be positive")
    engines = {}
    for entry in args.engine:
        name, separator, path = entry.partition("=")
        if not separator or not re.fullmatch(r"[a-zA-Z0-9_-]+", name) or name in engines:
            parser.error("engines must be unique name=path entries")
        engines[name] = Path(path).resolve()
    workloads = load_workloads(args.receipt, args.workload_dir, args.mode, args.case)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    (output / "raw").mkdir()
    metadata = dict(mode=args.mode, workloads=workloads, receipt_sha256=digest(args.receipt),
                    runner_sha256=digest(__file__), shared_runner_sha256=digest(Path(__file__).with_name("run.py")),
                    machine=machine_metadata(), engines={name: binary_metadata(path) for name, path in engines.items()},
                    repeat=repeat, timeout_seconds=args.timeout, order="serial rotating engine order per repetition",
                    metric="public compile API nanoseconds; no execution; excludes source IO/context creation/teardown" if args.mode == "compile" else "unmodified original V8 scores; higher is better",
                    ratio="after / before; compile elapsed time or original score, never inverse speedup",
                    process_wall_time="includes startup and teardown; separate from compile_ns and original scores")
    (output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    samples = []
    with (output / "samples.jsonl").open("w") as journal:
        for workload in workloads:
            for repetition in range(repeat):
                names = list(engines)
                offset = repetition % len(names)
                for name in names[offset:] + names[:offset]:
                    if digest(workload["path"]) != workload["sha256"]:
                        raise ValueError("workload changed during measurement")
                    if digest(engines[name]) != metadata["engines"][name]["sha256"]:
                        raise ValueError("binary changed during measurement")
                    prefix = output / "raw" / f"{workload['case']}-{name}-{repetition}"
                    sample = run_sample([str(engines[name]), workload["path"]], output, prefix, args.timeout)
                    status, measurements = admit(sample, workload, args.mode)
                    sample.update(case=workload["case"], engine=name, repetition=repetition, status=status,
                                  measurements=measurements, workload_sha256=workload["sha256"])
                    samples.append(sample)
                    journal.write(json.dumps(sample) + "\n")
                    journal.flush()
                    print(f"{prefix.name}: {status}", flush=True)
    summary, ratios = summarize(samples, list(engines))
    (output / "results.json").write_text(json.dumps(dict(metadata=metadata, summary=summary, ratios=ratios, samples=samples), indent=2) + "\n")
    return 0 if all(row["eligible"] for row in summary) else 1


if __name__ == "__main__":
    raise SystemExit(main())
