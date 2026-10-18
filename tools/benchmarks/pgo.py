#!/usr/bin/env python3
"""Build a PGO driver by replaying successful C compilations recorded by cc.py."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


REPO = Path(__file__).resolve().parents[2]


def training_arguments(arguments, output):
    # Training must not overwrite the original objects or dependency files.
    result = []
    arguments = iter(arguments)
    for argument in arguments:
        if argument in ("-o", "-MF", "-MT", "-MQ"):
            next(arguments)
        elif argument not in ("-MD", "-MMD", "-MP") and not argument.startswith("-ftime-trace"):
            result.append(argument)
    return [*result, "-o", str(output)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--records", type=Path, action="append", required=True,
                        help="cc.py record directory; repeat for different projects")
    parser.add_argument("--output", type=Path, required=True,
                        help="New directory for binaries, profiles, commands and logs")
    parser.add_argument("--toolchain", help="Rustup toolchain, e.g. 1.86.0")
    parser.add_argument("--clang", default="clang")
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    cargo = ["cargo"] + ([f"+{args.toolchain}"] if args.toolchain else [])
    rustc = ["rustc"] + ([f"+{args.toolchain}"] if args.toolchain else [])
    sysroot = Path(subprocess.check_output([*rustc, "--print", "sysroot"], text=True).strip())
    version = subprocess.check_output([*rustc, "-vV"], text=True)
    host = next(line.removeprefix("host: ") for line in version.splitlines()
                if line.startswith("host: "))
    profdata = sysroot / "lib" / "rustlib" / host / "bin" / "llvm-profdata"
    if not profdata.is_file():
        parser.error("install llvm-tools-preview for the selected Rust toolchain")
    if "CARGO_ENCODED_RUSTFLAGS" in os.environ:
        parser.error("unset CARGO_ENCODED_RUSTFLAGS; use RUSTFLAGS for common build flags")
    records = []
    for directory in args.records:
        for path in sorted(directory.glob("*.json")):
            if path.name.endswith(".trace.json"):
                continue
            record = json.loads(path.read_text())
            if record["kind"] == "compile" and record["returncode"] == 0:
                records.append(record)
    if not records:
        parser.error("no successful C compilation records found")
    clang = shutil.which(args.clang)
    if not clang:
        parser.error("Clang executable not found")
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    profiles = root / "profiles"
    profiles.mkdir()
    objects = root / "training-objects"
    objects.mkdir()
    env = dict(os.environ, DYNAMITE_CLANG=clang)
    env.pop("DYNAMITE_TIMINGS", None)
    env.pop("DYNAMITE_BACKEND", None)
    base_flags = env.get("RUSTFLAGS", "")
    metadata = {
        "rustc": version, "base_rustflags": base_flags, "clang": clang,
        "record_directories": [str(p.resolve()) for p in args.records],
        "training_compile_count": len(records), "jobs": args.jobs,
        "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
        "diff": subprocess.check_output(["git", "diff"], cwd=REPO, text=True),
    }

    def build(name, flag):
        target = root / f"{name}-target"
        command = [*cargo, "build", "--release", "--bin", "dynamite-cc",
                   "--target", host, "--target-dir", str(target)]
        build_env = dict(env, RUSTFLAGS=f"{base_flags} {flag}".strip())
        with (root / f"{name}-build.log").open("w") as log:
            subprocess.run(command, cwd=REPO, env=build_env,
                           stdout=log, stderr=subprocess.STDOUT, check=True)
        binary = root / f"dynamite-{name}"
        shutil.copy2(target / host / "release" / "dynamite-cc", binary)
        return binary

    instrumented = build("instrumented", f"-Cprofile-generate={profiles}")
    training = []
    for index, record in enumerate(records):
        training.append({"cwd": record["cwd"], "command": [str(instrumented),
                         *training_arguments(record["args"], objects / f"{index}.o")]})
    (root / "training.json").write_text(json.dumps(training, indent=2) + "\n")
    train_env = dict(env, LLVM_PROFILE_FILE=str(profiles / "%m-%p.profraw"))

    def train(item):
        index, invocation = item
        with (root / f"training-{index}.log").open("w") as log:
            subprocess.run(invocation["command"], cwd=invocation["cwd"], env=train_env,
                           stdout=log, stderr=subprocess.STDOUT, check=True)

    with ThreadPoolExecutor(max_workers=args.jobs) as workers:
        list(workers.map(train, enumerate(training)))
    merged = root / "merged.profdata"
    raw_profiles = sorted(profiles.glob("*.profraw"))
    if not raw_profiles:
        raise RuntimeError("training produced no profiles")
    subprocess.run([str(profdata), "merge", "-o", str(merged),
                    *map(str, raw_profiles)], check=True)
    optimized = build("pgo", f"-Cprofile-use={merged}")
    metadata["binary"] = str(optimized)
    metadata["binary_sha256"] = hashlib.sha256(optimized.read_bytes()).hexdigest()
    (root / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(optimized)


if __name__ == "__main__":
    main()
