#!/usr/bin/env python3
"""Build and screen pinned GNU gzip 1.14 end to end.

This is VM wall-clock screening, never a PMU or bare-metal claim.  The script
verifies the exact archive digest, builds the complete upstream project, runs
its 30-test suite, checks compressed/decompressed bytes, captures disassembly,
and records paired randomized timings for deterministic source/mixed corpora.
An optional same-compiler environment control exposes treatment/control cases,
aggregate ratios, best gain, and worst regression in one retained report.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import random
import shutil
import statistics
import subprocess
import tarfile
import tempfile
import time
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

VERSION = "1.14"
ARCHIVE_URL = "https://ftp.gnu.org/pub/gnu/gzip/gzip-1.14.tar.xz"
ARCHIVE_SHA256 = "01a7b881bd220bfdf615f97b8718f80bdfd3f6add385b993dcf6efd14e8c0ac6"
ARCHPKGBUILDS_SELECTOR = "packages/gzip/PKGBUILD pkgver=1.14"
CORPUS_BYTES = 8 * 1024 * 1024
SEED = 0x475A4950313134


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def run(command: list[str], *, cwd: Path | None = None,
        env: dict[str, str] | None = None, timeout: int = 600,
        stdout=None, stdin=None) -> subprocess.CompletedProcess:
    result = subprocess.run(
        command, cwd=str(cwd) if cwd else None, env=env,
        stdin=stdin, stdout=stdout if stdout is not None else subprocess.PIPE,
        stderr=subprocess.PIPE, text=stdout is None and stdin is None,
        timeout=timeout, check=False,
    )
    if result.returncode:
        stderr = result.stderr if isinstance(result.stderr, str) else result.stderr.decode(errors="replace")
        raise RuntimeError(f"command failed ({result.returncode}): {' '.join(command)}\n{stderr[-4000:]}")
    return result


def compiler_version(path: str) -> str:
    result = subprocess.run([path, "--version"], capture_output=True, text=True, timeout=20)
    return next((line for line in (result.stdout + result.stderr).splitlines() if line.strip()), "unknown")


def get_archive(value: str | None, cache: Path) -> Path:
    if value:
        archive = Path(value).resolve()
    else:
        cache.mkdir(parents=True, exist_ok=True)
        archive = cache / f"gzip-{VERSION}.tar.xz"
        if not archive.exists():
            tmp = archive.with_suffix(".download")
            urllib.request.urlretrieve(ARCHIVE_URL, tmp)
            tmp.replace(archive)
    if not archive.is_file():
        raise SystemExit(f"archive not found: {archive}")
    actual = sha256(archive)
    if actual != ARCHIVE_SHA256:
        raise SystemExit(f"archive SHA-256 mismatch: expected {ARCHIVE_SHA256}, got {actual}")
    return archive


def source_corpus(source: Path, output: Path) -> None:
    selected = sorted(
        p for p in source.rglob("*")
        if p.is_file() and p.suffix in {".c", ".h", ".in", ".texi"}
    )
    material = bytearray()
    for path in selected:
        rel = path.relative_to(source).as_posix().encode()
        material.extend(b"\n/* FILE ")
        material.extend(rel)
        material.extend(b" */\n")
        material.extend(path.read_bytes())
    if not material:
        raise RuntimeError("source corpus selection is empty")
    with output.open("wb") as f:
        remaining = CORPUS_BYTES
        while remaining:
            chunk = material[:remaining]
            f.write(chunk)
            remaining -= len(chunk)


def mixed_corpus(output: Path) -> None:
    structured = (
        b'{"level":"info","component":"compiler","message":"building gzip object",'
        b'"target":"x86-64-v3","status":"ok"}\n'
        b'<file name="manual.texi"><section>Compression workload</section></file>\n'
    )
    with output.open("wb") as f:
        counter = 0
        while f.tell() < CORPUS_BYTES:
            # Alternate a compressible build-log block and deterministic binary
            # digest material. This adapts the package recipe's text/binary mix
            # without /dev/urandom or timestamps.
            f.write(structured * 32)
            for _ in range(128):
                f.write(hashlib.sha256(f"gzip-1.14:{counter}".encode()).digest())
                counter += 1
            if f.tell() > CORPUS_BYTES:
                f.truncate(CORPUS_BYTES)
                break


def parse_check_summary(log: str) -> dict[str, int]:
    result: dict[str, int] = {}
    for key in ("TOTAL", "PASS", "SKIP", "XFAIL", "FAIL", "XPASS", "ERROR"):
        for line in log.splitlines():
            if line.startswith(f"# {key}:"):
                result[key.lower()] = int(line.split(":", 1)[1].strip())
                break
    return result


def build_one(key: str, cc: str, source: Path, root: Path,
              artifacts: Path, gcc_include: str,
              extra_env: dict[str, str] | None = None) -> dict:
    build = root / f"obj-{key}"
    build.mkdir()
    flags = "-O3 -march=x86-64-v3"
    if key.startswith("lccc"):
        flags += f" -I{gcc_include}"
    env = os.environ.copy()
    env.update(CC=cc, CFLAGS=flags)
    if extra_env:
        env.update(extra_env)
    configure = [str(source / "configure"), "--disable-dependency-tracking"]
    configured = run(configure, cwd=build, env=env, timeout=600)
    built = run(["make", "-j2"], cwd=build, env=env, timeout=900)
    checked = run(["make", "check", "-j2"], cwd=build, env=env, timeout=1200)
    (artifacts / f"configure-{key}.log").write_text(
        (configured.stdout or "") + (configured.stderr or ""), errors="replace")
    (artifacts / f"build-{key}.log").write_text(
        (built.stdout or "") + (built.stderr or ""), errors="replace")
    check_text = (checked.stdout or "") + (checked.stderr or "")
    (artifacts / f"check-{key}.log").write_text(check_text, errors="replace")
    binary = build / "gzip"
    copied = artifacts / f"gzip-{key}"
    shutil.copy2(binary, copied)
    objdump = shutil.which("objdump")
    if objdump:
        disasm = run([objdump, "-drwC", str(binary)], timeout=120)
        (artifacts / f"gzip-{key}.objdump.txt").write_text(disasm.stdout)
    size_result = run([shutil.which("size") or "size", str(copied)])
    (artifacts / f"gzip-{key}.size.txt").write_text(size_result.stdout)
    summary = parse_check_summary(check_text)
    if summary.get("total") != 30 or summary.get("pass") != 30 or summary.get("fail", 0):
        raise RuntimeError(f"{key}: unexpected gzip test summary {summary}")
    return {
        "key": key,
        "cc": cc,
        "compiler_version": compiler_version(cc),
        "flags": flags,
        "environment": dict(sorted((extra_env or {}).items())),
        "binary": str(copied),
        "binary_sha256": sha256(copied),
        "binary_bytes": copied.stat().st_size,
        "check": summary,
    }


def gzip_to_file(binary: Path, source: Path, output: Path, level: int) -> None:
    with source.open("rb") as inp, output.open("wb") as out:
        run([str(binary), "-c", f"-{level}"], stdin=inp, stdout=out, timeout=180)


def gunzip_to_file(binary: Path, source: Path, output: Path) -> None:
    with source.open("rb") as inp, output.open("wb") as out:
        run([str(binary), "-dc"], stdin=inp, stdout=out, timeout=180)


def timed(command: list[str], input_path: Path, cpu: int) -> int:
    prefix = ["taskset", "-c", str(cpu)] if shutil.which("taskset") else []
    start = time.perf_counter_ns()
    with input_path.open("rb") as inp, open(os.devnull, "wb") as out:
        run(prefix + command, stdin=inp, stdout=out, timeout=180)
    return time.perf_counter_ns() - start


def summarize(samples: dict[str, dict[str, list[int]]], baseline: str) -> tuple[dict, list[str]]:
    summary: dict[str, dict] = {}
    lines = [
        "| case | compiler | median ms | ratio to gcc | best ms | worst ms |",
        "|---|---|---:|---:|---:|---:|",
    ]
    ratios: dict[str, list[float]] = {k: [] for k in next(iter(samples.values()))}
    for case, compilers in samples.items():
        base_median = statistics.median(compilers[baseline])
        summary[case] = {}
        for key, values in compilers.items():
            med = statistics.median(values)
            ratio = med / base_median
            ratios[key].append(ratio)
            summary[case][key] = {
                "samples_ns": values,
                "median_ns": med,
                "best_ns": min(values),
                "worst_ns": max(values),
                "ratio_to_gcc": ratio,
            }
            lines.append(
                f"| {case} | {key} | {med/1e6:.3f} | {ratio:.5f} | "
                f"{min(values)/1e6:.3f} | {max(values)/1e6:.3f} |"
            )
    lines += ["", "| compiler | arithmetic mean ratio | geometric mean ratio |",
              "|---|---:|---:|"]
    for key, values in ratios.items():
        ar = statistics.mean(values)
        geo = math.exp(statistics.mean(math.log(v) for v in values))
        lines.append(f"| {key} | {ar:.6f} | {geo:.6f} |")
    return summary, lines


def compare_treatment_control(
    summary: dict[str, dict], treatment: str, control: str,
) -> tuple[dict, list[str]]:
    """Expose every treatment/control case plus aggregate and extrema.

    Ratios below 1.0 favor the treatment. Medians are paired by case; raw
    randomized samples remain in the main summary for independent review.
    """
    cases: dict[str, dict] = {}
    ratios: list[float] = []
    lines = [
        "## LCCC treatment versus kill-switch control",
        "",
        "VM wall-clock screening only; ratio < 1 favors treatment.",
        "",
        "| case | treatment ms | control ms | treatment/control | gain % |",
        "|---|---:|---:|---:|---:|",
    ]
    for case, records in summary.items():
        treatment_ns = records[treatment]["median_ns"]
        control_ns = records[control]["median_ns"]
        ratio = treatment_ns / control_ns
        ratios.append(ratio)
        cases[case] = {
            "treatment_median_ns": treatment_ns,
            "control_median_ns": control_ns,
            "treatment_to_control": ratio,
            "gain_percent": (1.0 - ratio) * 100.0,
        }
        lines.append(
            f"| {case} | {treatment_ns/1e6:.3f} | {control_ns/1e6:.3f} | "
            f"{ratio:.6f} | {(1.0-ratio)*100.0:+.3f} |"
        )
    best = min(cases, key=lambda k: cases[k]["treatment_to_control"])
    worst = max(cases, key=lambda k: cases[k]["treatment_to_control"])
    aggregates = {
        "arithmetic_mean_ratio": statistics.mean(ratios),
        "geometric_mean_ratio": math.exp(statistics.mean(math.log(v) for v in ratios)),
        "best_gain_case": best,
        "best_gain_ratio": cases[best]["treatment_to_control"],
        "worst_regression_case": worst,
        "worst_regression_ratio": cases[worst]["treatment_to_control"],
    }
    lines += [
        "",
        f"- Arithmetic mean ratio: **{aggregates['arithmetic_mean_ratio']:.6f}**",
        f"- Geometric mean ratio: **{aggregates['geometric_mean_ratio']:.6f}**",
        f"- Best case: `{best}` **{aggregates['best_gain_ratio']:.6f}**",
        f"- Worst case: `{worst}` **{aggregates['worst_regression_ratio']:.6f}**",
    ]
    return {"cases": cases, "aggregates": aggregates}, lines


def parse_env_assignments(values: list[str]) -> dict[str, str]:
    env: dict[str, str] = {}
    for value in values:
        if "=" not in value:
            raise SystemExit(f"control environment must be NAME=VALUE, got {value!r}")
        name, assigned = value.split("=", 1)
        if not name or not name.replace("_", "a").isalnum() or name[0].isdigit():
            raise SystemExit(f"invalid environment variable name: {name!r}")
        env[name] = assigned
    return env


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--archive", help="local gzip-1.14.tar.xz (otherwise pinned URL/cache)")
    p.add_argument("--cache", type=Path, default=Path.home() / ".cache/lccc-workloads")
    p.add_argument("--lccc", default="target/fastbuild/lccc")
    p.add_argument(
        "--lccc-control-env", action="append", default=[], metavar="NAME=VALUE",
        help="also build a same-compiler kill-switch control with this environment; repeatable",
    )
    p.add_argument("--artifact-dir", type=Path, required=True)
    p.add_argument("--rounds", type=int, default=9)
    p.add_argument("--warmups", type=int, default=2)
    args = p.parse_args()
    control_env = parse_env_assignments(args.lccc_control_env)

    archive = get_archive(args.archive, args.cache)
    lccc = str(Path(args.lccc).resolve())
    gcc = shutil.which("gcc")
    if not Path(lccc).is_file() or not gcc:
        raise SystemExit("both --lccc and gcc are required")
    gcc_include = subprocess.check_output([gcc, "-print-file-name=include"], text=True).strip()
    artifacts = args.artifact_dir.resolve()
    artifacts.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="lccc-gzip-1.14.") as td:
        root = Path(td)
        with tarfile.open(archive) as tf:
            tf.extractall(root, filter="data")
        source = root / f"gzip-{VERSION}"
        source_input = root / "source-corpus.bin"
        mixed_input = root / "mixed-corpus.bin"
        source_corpus(source, source_input)
        mixed_corpus(mixed_input)
        inputs = {"source": source_input, "mixed": mixed_input}
        input_meta = {k: {"bytes": v.stat().st_size, "sha256": sha256(v)}
                      for k, v in inputs.items()}

        builds = {
            "lccc": build_one("lccc", lccc, source, root, artifacts, gcc_include),
            "gcc": build_one("gcc", gcc, source, root, artifacts, gcc_include),
        }
        if control_env:
            builds["lccc-control"] = build_one(
                "lccc-control", lccc, source, root, artifacts, gcc_include, control_env,
            )
        binaries = {k: Path(v["binary"]) for k, v in builds.items()}

        # Correctness: complete compressed streams must be bit-identical, and
        # every binary must restore the exact pinned input bytes.
        stream_meta: dict[str, dict] = {}
        compressed: dict[tuple[str, str], Path] = {}
        for corpus, inp in inputs.items():
            for level in (1, 6, 9):
                case = f"compress-{corpus}-l{level}"
                digests = {}
                for key, binary in binaries.items():
                    out = root / f"{case}-{key}.gz"
                    gzip_to_file(binary, inp, out, level)
                    compressed[(corpus, key)] = out if level == 6 else compressed.get((corpus, key), out)
                    digests[key] = sha256(out)
                if len(set(digests.values())) != 1:
                    raise RuntimeError(f"non-identical gzip streams for {case}: {digests}")
                stream_meta[case] = {"sha256": next(iter(digests.values())), "compiler_sha256": digests}
        for corpus, inp in inputs.items():
            reference = root / f"decompress-{corpus}-input.gz"
            gzip_to_file(binaries["gcc"], inp, reference, 6)
            for key, binary in binaries.items():
                restored = root / f"restored-{corpus}-{key}"
                gunzip_to_file(binary, reference, restored)
                if sha256(restored) != input_meta[corpus]["sha256"]:
                    raise RuntimeError(f"{key}: decompression mismatch for {corpus}")

        cases = {
            "compress-source-l1": (lambda b: [str(b), "-c", "-1"], source_input),
            "compress-source-l6": (lambda b: [str(b), "-c", "-6"], source_input),
            "compress-source-l9": (lambda b: [str(b), "-c", "-9"], source_input),
            "compress-mixed-l6": (lambda b: [str(b), "-c", "-6"], mixed_input),
        }
        # Decompress one exact GCC stream for both binaries.
        decomp = root / "timed-source-l6.gz"
        gzip_to_file(binaries["gcc"], source_input, decomp, 6)
        cases["decompress-source-l6"] = (lambda b: [str(b), "-dc"], decomp)

        affinity = sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else [0]
        cpu = affinity[0]
        samples = {case: {key: [] for key in builds} for case in cases}
        rng = random.Random(SEED)
        for case, (command_for, inp) in cases.items():
            for _ in range(args.warmups):
                order = list(builds); rng.shuffle(order)
                for key in order:
                    timed(command_for(binaries[key]), inp, cpu)
            for _ in range(args.rounds):
                order = list(builds); rng.shuffle(order)
                for key in order:
                    samples[case][key].append(timed(command_for(binaries[key]), inp, cpu))

        summary, table = summarize(samples, "gcc")
        treatment_control = None
        treatment_table: list[str] = []
        if control_env:
            treatment_control, treatment_table = compare_treatment_control(
                summary, "lccc", "lccc-control",
            )
        evidence = {
            "schema": 2,
            "date_utc": datetime.now(timezone.utc).isoformat(),
            "evidence_class": "VM wall-clock screening; no PMU",
            "archive": {"url": ARCHIVE_URL, "path": str(archive),
                        "sha256": ARCHIVE_SHA256,
                        "archpkgbuilds_selector": ARCHPKGBUILDS_SELECTOR,
                        "signature_verified": False},
            "corpora": input_meta,
            "builds": builds,
            "correctness_streams": stream_meta,
            "cpu": cpu,
            "rounds": args.rounds,
            "warmups": args.warmups,
            "summary": summary,
            "treatment_vs_control": treatment_control,
        }
        (artifacts / "results.json").write_text(json.dumps(evidence, indent=2) + "\n")
        report = [
            "# GNU gzip 1.14 end-to-end workload",
            "",
            "**Evidence:** VM wall-clock screening only; no PMU or bare-metal claim.",
            "",
            f"Archive SHA-256: `{ARCHIVE_SHA256}`; upstream test suites: 30/30 for each build.",
            f"Pinned CPU: `{cpu}`; rounds: {args.rounds}; warmups: {args.warmups}.",
            "",
            *table,
            "",
            *treatment_table,
            "" if treatment_table else "",
            "The current archpkgbuilds recipe checksum differs from the repeatedly fetched",
            "archive digest above; the archive signature was not verified in this run.",
        ]
        (artifacts / "report.md").write_text("\n".join(report) + "\n")
        print("\n".join(report))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
