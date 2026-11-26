#!/usr/bin/env python3
"""Interleave two *validated* SQLite/LCCC builds; correctness before timing.

The default A/B is the same LCCC fastbuild with ptr_delta_phi enabled versus
CCC_DISABLE_PASSES=ptr_delta_phi. This is a VM screen, not native-CPU/PMU proof.
The input artifact dirs must each contain a PASS manifest and retained builds.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import statistics
import subprocess
import tempfile
import time

from summarize import parse_run

IDENTITY_KEYS = (
    "version", "source_zip_sha256", "fossil_manifest_uuid", "gcc_version",
    "archpkgbuilds_commit", "recipe_pkbuild_sha256", "archpkgbuilds_lemon_patch_sha256",
    "lemon_patch_status", "applied_patch_sha256", "lccc_compiler_sha256",
    "cflags", "cppflags", "build_jobs", "speedtest_size", "speedtest_repeats",
)


def input_build(path: pathlib.Path) -> tuple[dict, pathlib.Path]:
    manifest = json.loads((path / "manifest.json").read_text())
    if manifest.get("status") != "PASS":
        raise ValueError(f"not a validated SQLite workload: {path}")
    work = pathlib.Path((path / "work-path.txt").read_text().strip())
    if not (work / "build-lccc/speedtest1").is_file() or not (work / "build-lccc/sqlite3").is_file():
        raise ValueError(f"retained SQLite binaries missing: {work}")
    return manifest, work / "build-lccc"


def identity(a: dict, b: dict, sets: list[str]) -> None:
    for key in IDENTITY_KEYS:
        if not a.get(key) or a[key] != b.get(key):
            raise ValueError(f"different or missing {key}: {a.get(key)!r} / {b.get(key)!r}")
    if a.get("lccc_disable_passes") != "" or b.get("lccc_disable_passes") != "ptr_delta_phi":
        raise ValueError("expected enabled A and ptr_delta_phi-only disabled B")
    for testset in sets:
        left = a["speedtest1"][testset]
        right = b["speedtest1"][testset]
        for key in ("verification_hash", "verified_bytes", "test_count"):
            if left[key] != right[key]:
                raise ValueError(f"upstream verification mismatch: {testset}/{key}")
        if left["runs"]["gcc"][0]["tests"] != right["runs"]["gcc"][0]["tests"]:
            raise ValueError(f"different upstream test identities: {testset}")


def require_silent(stderr: bytes, log: pathlib.Path) -> None:
    # Verification hashes and zero exit codes do not excuse runtime warnings.
    if stderr:
        raise ValueError(f"unexpected runtime stderr: {log}")


def validate_run(run: dict, expected: dict, log: pathlib.Path) -> None:
    # The manifest stores JSON lists; parse_run returns pairs of Python tuples.
    # Normalize ONLY the representation, not test numbers/names or the hash.
    expected_ids = [tuple(row) for row in expected["runs"]["gcc"][0]["tests"]]
    if (run["verification_hash"] != expected["verification_hash"]
            or run["verification_bytes"] != expected["verified_bytes"]
            or run["tests"] != expected_ids
            or len(run["tests"]) != expected["test_count"]):
        raise ValueError(f"speedtest hash or test identities differ: {log}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--enabled", type=pathlib.Path, required=True)
    ap.add_argument("--disabled", type=pathlib.Path, required=True)
    ap.add_argument("--sets", nargs="+", default="main cte rtree orm json fp".split())
    ap.add_argument("--repeats", type=int, default=7)
    ap.add_argument("--out", type=pathlib.Path, required=True)
    args = ap.parse_args()
    if args.repeats < 1 or not args.sets or len(args.sets) != len(set(args.sets)):
        ap.error("positive repeats and distinct nonempty testsets required")
    a, a_bin = input_build(args.enabled)
    b, b_bin = input_build(args.disabled)
    identity(a, b, args.sets)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    raw = args.out.parent / (args.out.stem + "-raw")
    raw.mkdir(exist_ok=True)
    results = {}
    with tempfile.TemporaryDirectory(prefix="sqlite-paired-", dir=args.out.parent) as td:
        for testset in args.sets:
            expected = a["speedtest1"][testset]
            expected_ids = [tuple(row) for row in expected["runs"]["gcc"][0]["tests"]]
            samples = {"enabled": [], "disabled": []}
            for rep in range(args.repeats):
                order = ("enabled", "disabled") if rep % 2 == 0 else ("disabled", "enabled")
                for key in order:
                    exe = a_bin if key == "enabled" else b_bin
                    db = pathlib.Path(td) / f"{testset}-{rep}-{key}.db"
                    log = raw / f"{testset}-{rep}-{key}.txt"
                    t0 = time.monotonic()
                    proc = subprocess.run(
                        [str(exe / "speedtest1"), "--size", str(a["speedtest_size"]),
                         "--nosync", "--testset", testset, "--verify", str(db)],
                        capture_output=True, timeout=180,
                    )
                    elapsed = time.monotonic() - t0
                    log.write_bytes(proc.stdout)
                    err = log.with_suffix(".err")
                    err.write_bytes(proc.stderr)
                    if proc.returncode:
                        raise RuntimeError(f"speedtest failed for {log}: rc={proc.returncode}")
                    require_silent(proc.stderr, err)
                    run = parse_run(log)
                    validate_run(run, expected, log)
                    integrity = subprocess.run(
                        [str(exe / "sqlite3"), "-batch", "-bail", str(db),
                         "PRAGMA integrity_check;"], capture_output=True, timeout=120,
                    )
                    if integrity.returncode or integrity.stdout.strip() != b"ok" or integrity.stderr:
                        raise ValueError(f"database integrity_check failed or wrote stderr: {db}")
                    db.unlink()  # do not retain 84 large databases on a 21 GiB VM
                    samples[key].append({
                        "round": rep, "vm_seconds": run["total_seconds"],
                        "wall_seconds": elapsed, "stdout_sha256": run["stdout_sha256"],
                    })
            left = [run["vm_seconds"] for run in samples["enabled"]]
            right = [run["vm_seconds"] for run in samples["disabled"]]
            ratio = statistics.median(right) / statistics.median(left)
            paired = statistics.median(y / x for x, y in zip(left, right))
            results[testset] = {
                "hash": expected["verification_hash"], "test_count": expected["test_count"],
                "enabled_median_vm_seconds": statistics.median(left),
                "disabled_median_vm_seconds": statistics.median(right),
                "disabled_over_enabled_vm_ratio": ratio,
                "median_paired_ratio": paired, "samples": samples,
            }
            print(f"{testset}: {len(expected_ids)} tests, same verification hash; "
                  f"disabled/enabled VM median {ratio:.3f}x "
                  f"(paired {paired:.3f}x)", flush=True)
    report = {
        "schema": 1, "status": "PASS", "repeats": args.repeats,
        "order": "alternating AB/BA; independent file databases; integrity per run",
        "caveat": "VM times only; no PMU or native target-hardware speed claim",
        "lccc_compiler_sha256": a["lccc_compiler_sha256"],
        "executables": {
            "enabled": {"path": str(a_bin / "speedtest1"),
                        "sha256": hashlib.sha256((a_bin / "speedtest1").read_bytes()).hexdigest()},
            "disabled": {"path": str(b_bin / "speedtest1"),
                         "sha256": hashlib.sha256((b_bin / "speedtest1").read_bytes()).hexdigest()},
        },
        "sets": results,
    }
    args.out.write_text(json.dumps(report, indent=2) + "\n")
    print(f"PASS: {args.out}; all hash, test-identity and database-integrity gates matched")


if __name__ == "__main__":
    main()
