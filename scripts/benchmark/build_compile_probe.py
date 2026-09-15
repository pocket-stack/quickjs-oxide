"""Build the same public compile probe against an explicit checkout and feature set."""
import argparse
import json
import os
import subprocess
import tomllib
from pathlib import Path
from run import digest


def manifest_files(receipt):
    files = receipt.get("files", receipt.get("source_files"))
    if not isinstance(files, dict) or not files:
        raise ValueError("source manifest must contain nonempty files or source_files hashes")
    for name, sha256 in files.items():
        path = Path(name)
        if path.is_absolute() or ".." in path.parts or name != path.as_posix():
            raise ValueError(f"unsafe source manifest path: {name}")
        if not isinstance(sha256, str) or len(sha256) != 64 or any(c not in "0123456789abcdef" for c in sha256):
            raise ValueError(f"invalid source hash: {name}")
    return files


def validate_frozen_source(repo, files):
    # Frozen inputs must be a complete export. A new unrecorded Rust module or
    # Cargo config is as significant as an edit to a recorded source file.
    actual = set()
    for directory, directories, names in os.walk(repo):
        if Path(directory) == repo:
            directories[:] = [name for name in directories if name not in {".git", "target"}]
        if any((Path(directory) / name).is_symlink() for name in directories):
            raise ValueError("frozen source directory symlinks are not supported")
        for name in names:
            path = Path(directory) / name
            if not path.resolve().is_relative_to(repo):
                raise ValueError(f"source link escapes export: {path}")
            actual.add(path.relative_to(repo).as_posix())
    if actual != files.keys():
        raise ValueError(f"source inventory differs from manifest: missing={sorted(files.keys() - actual)}, extra={sorted(actual - files.keys())}")
    for name, sha256 in files.items():
        if digest(repo / name) != sha256:
            raise ValueError(f"frozen source changed: {name}")


def exact_git_root(repo):
    result = subprocess.run(["git", "-C", str(repo), "rev-parse", "--show-toplevel"],
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    return result.returncode == 0 and Path(result.stdout.strip()).resolve() == repo


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--source-manifest", type=Path,
                        help="complete frozen export identity; mandatory when --repo is not an exact Git checkout root")
    parser.add_argument("--stack-vm", action="store_true")
    parser.add_argument("--profiling", action="store_true")
    args = parser.parse_args()
    repo, output = args.repo.resolve(), args.output.resolve()
    source_receipt = None
    source_manifest_sha256 = None
    if args.source_manifest:
        if output.is_relative_to(repo):
            parser.error("frozen build output must be outside the source export")
        source_manifest_sha256 = digest(args.source_manifest)
        source_receipt = json.loads(args.source_manifest.read_text())
        frozen_files = manifest_files(source_receipt)
        validate_frozen_source(repo, frozen_files)
    elif not exact_git_root(repo):
        parser.error("--repo is not an exact Git checkout root; supply --source-manifest (ancestor Git metadata is not source identity)")
    output.mkdir(parents=True, exist_ok=False)
    (output / "src").mkdir()
    source = Path(__file__).resolve().parents[2] / "apps/cli/examples/compile_probe.rs"
    (output / "src/main.rs").write_bytes(source.read_bytes())
    probe_source_sha256 = digest(output / "src/main.rs")
    features = [name for enabled, name in [(args.stack_vm, "stack-vm"), (args.profiling, "profiling")] if enabled]
    manifest = '\n'.join([
        '[package]', 'name="oxide-compile-probe"', 'version="0.0.0"', 'edition="2024"',
        '[workspace]', '[features]', 'profiling=[]', '[dependencies]',
        f'quickjs-oxide={{path={json.dumps(str(repo))},default-features=false,features={json.dumps(features)}}}',
        f'quickjs-oxide-host={{path={json.dumps(str(repo / "adapters/native"))}}}', '',
    ])
    (output / "Cargo.toml").write_text(manifest)
    (output / "Cargo.lock").write_bytes((repo / "Cargo.lock").read_bytes())
    command = ["cargo", "build", "--release", "--manifest-path", str(output / "Cargo.toml"), "--target-dir", str(output / "target")]
    if args.profiling:
        command += ["--features", "profiling"]
    with (output / "build.log").open("w") as log:
        subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
    if digest(output / "src/main.rs") != probe_source_sha256:
        raise ValueError("compile probe source changed during build")
    if source_receipt is not None:
        if digest(args.source_manifest) != source_manifest_sha256:
            raise ValueError("source manifest changed during build")
        validate_frozen_source(repo, frozen_files)
    pinned = {(p["name"], p["version"], p.get("source")): p.get("checksum")
              for p in tomllib.loads((repo / "Cargo.lock").read_text())["package"]}
    for dependency in tomllib.loads((output / "Cargo.lock").read_text())["package"]:
        if dependency.get("source"):
            key = dependency["name"], dependency["version"], dependency["source"]
            if key not in pinned or pinned[key] != dependency.get("checksum"):
                raise ValueError("compile probe dependency drift; binary not admitted")
    binary = output / "target/release/oxide-compile-probe"
    if source_receipt is not None:
        (output / "source-manifest.json").write_bytes(args.source_manifest.read_bytes())
        source_identity = dict(kind="frozen-export", manifest_sha256=source_manifest_sha256,
                               manifest_file="source-manifest.json", files_validated=len(frozen_files),
                               validated_before_and_after_build=True)
        dependency_commit = source_receipt.get("base", source_receipt.get("parent"))
        source_patch_sha256 = None
    else:
        tracked_diff = subprocess.check_output(["git", "-C", str(repo), "diff", "HEAD", "--binary"])
        (output / "source.patch").write_bytes(tracked_diff)
        dependency_commit = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip()
        source_patch_sha256 = digest(output / "source.patch")
        source_identity = dict(kind="git-checkout", note="patch covers tracked files; stage receipt must additionally hash untracked implementation files")
    metadata = dict(command=command, repository=str(repo), binary=str(binary), binary_sha256=digest(binary),
                    source_sha256=probe_source_sha256, cargo_lock_sha256=digest(output / "Cargo.lock"),
                    dependency_commit=dependency_commit, source_identity=source_identity,
                    source_patch_sha256=source_patch_sha256, features=features,
                    rustc=subprocess.check_output(["rustc", "-vV"], text=True),
                    rustflags=os.environ.get("RUSTFLAGS"), encoded_rustflags=os.environ.get("CARGO_ENCODED_RUSTFLAGS"))
    (output / "build.json").write_text(json.dumps(metadata, indent=2) + "\n")
    binary.with_suffix(".build.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(binary)


if __name__ == "__main__":
    main()
