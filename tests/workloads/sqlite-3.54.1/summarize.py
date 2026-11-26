#!/usr/bin/env python3
"""Check upstream speedtest1 semantic hashes and retain paired VM timings.

The hashes and test identities are the gate; elapsed times are noisy research
screening only, not evidence for Raptor Lake hardware performance.
"""
from __future__ import annotations

import hashlib
import json
import os
import pathlib
import re
import shutil
import statistics
import subprocess
import sys
from collections import defaultdict

TOTAL = re.compile(r"^\s*TOTAL\.*\s+(\d+\.\d{3})s\s*$", re.M)
VERIFY = re.compile(r"^Verification Hash: (\d+) ([0-9a-f]{48})\s*$", re.M)
TEST = re.compile(r"^\s*(\d+) - (.*?)\.{2,}\s+(\d+\.\d{3})s\s*$", re.M)


def parse_run(path: pathlib.Path) -> dict:
    text = path.read_text()
    total, verification = TOTAL.search(text), VERIFY.search(text)
    tests = TEST.findall(text)
    if not total or not verification or not tests:
        raise ValueError(f"missing total/hash/test rows in {path}")
    identities = [(int(number), name.strip()) for number, name, _ in tests]
    if len(set(identities)) != len(identities):
        raise ValueError(f"duplicate tests in {path}")
    return {
        "total_seconds": float(total[1]),
        "verification_bytes": int(verification[1]),
        "verification_hash": verification[2],
        "tests": identities,
        "test_seconds": [float(duration) for _, _, duration in tests],
        "stdout_sha256": hashlib.sha256(text.encode()).hexdigest(),
    }


def version(executable: str) -> str:
    return subprocess.check_output([executable, "--version"], text=True).splitlines()[0]


def compiler_sha256(executable: str) -> str:
    # Stream the large fastbuild binary: avoid a 146 MB Python allocation on
    # the constrained compilation VM simply to record its identity.
    digest = hashlib.sha256()
    with pathlib.Path(executable).open("rb") as binary:
        for chunk in iter(lambda: binary.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def summarize(artifact_dir: pathlib.Path, sets: list[str], repeats: int) -> dict:
    results = {}
    for testset in sets:
        grouped: dict[str, list] = defaultdict(list)
        identities = None
        baseline_hash = None
        for repeat in range(1, repeats + 1):
            for key in ("gcc", "lccc"):
                path = artifact_dir / f"bench-{testset}-{repeat}-{key}.txt"
                run = parse_run(path)
                if identities is None:
                    identities = run["tests"]
                if run["tests"] != identities:
                    raise ValueError(f"different speedtest1 test identities: {path}")
                pair = (run["verification_bytes"], run["verification_hash"])
                if baseline_hash is None:
                    baseline_hash = pair
                if pair != baseline_hash:
                    raise ValueError(f"speedtest1 result mismatch: {path}: {pair} != {baseline_hash}")
                grouped[key].append({"run": repeat, **run})
        gcc_median = statistics.median(r["total_seconds"] for r in grouped["gcc"])
        lccc_median = statistics.median(r["total_seconds"] for r in grouped["lccc"])
        results[testset] = {
            "test_count": len(identities),
            "verified_bytes": baseline_hash[0],
            "verification_hash": baseline_hash[1],
            "runs": dict(grouped),
            "gcc_median_seconds": gcc_median,
            "lccc_median_seconds": lccc_median,
            "lccc_over_gcc_median_ratio_vm_only": lccc_median / gcc_median if gcc_median else None,
        }
    return results


def main(args: list[str]) -> int:
    if len(args) != 18:
        raise SystemExit("usage: summarize.py ART VERSION ARCHIVE_SHA FOSSIL_ID "
                         "RECIPE_COMMIT RECIPE_SHA RECIPE_PATCH_SHA STATUS APPLIED_SHA "
                         "LCCC LCCC_LD GCC CFLAGS CPPFLAGS JOBS SPEED_SIZE REPEATS SETS")
    (art, release, archive_sha, fossil_id, recipe_commit, recipe_sha,
     recipe_patch_sha, patch_status, applied_sha, lccc, lccc_ld, gcc, cflags,
     cppflags, jobs, size, reps, sets_string) = args
    art = pathlib.Path(art)
    sets = sets_string.split()
    if not sets or len(sets) != len(set(sets)):
        raise ValueError("empty or duplicate speedtest1 testsets")
    results = summarize(art, sets, int(reps))
    payload = {
        "schema": 1, "status": "PASS", "version": release,
        "source": "sqlite/sqlite GitHub release tag (mirror of Fossil release)",
        "source_zip_sha256": archive_sha, "fossil_manifest_uuid": fossil_id,
        "archpkgbuilds_commit": recipe_commit, "recipe_pkbuild_sha256": recipe_sha,
        "archpkgbuilds_lemon_patch_sha256": recipe_patch_sha,
        "lemon_patch_status": patch_status, "applied_patch_sha256": applied_sha,
        "lccc": str(pathlib.Path(lccc).resolve()), "lccc_version": version(lccc),
        "lccc_compiler_sha256": compiler_sha256(lccc),
        "lccc_disable_passes": os.environ.get("CCC_DISABLE_PASSES", ""),
        "lccc_ld": str(pathlib.Path(lccc_ld).resolve()), "lccc_ld_version": version(lccc_ld),
        "gcc": str(pathlib.Path(shutil.which(gcc) or gcc).resolve()),
        "gcc_version": version(gcc), "cflags": cflags, "cppflags": cppflags.strip(),
        "build_jobs": int(jobs), "speedtest_size": int(size), "speedtest_repeats": int(reps),
        "sql_cases": ["relational", "features"],
        "sql_reference": "GCC 16.2 same flags and sources; LCCC driver vs standalone linker",
        "sqlite_tcl_tests": "not run (no tclsh installed); upstream speedtest1 --verify and SQL/integrity tests run",
        "timing_caveat": "VM wall time only; not target Raptor Lake/PMU evidence",
        "speedtest1": results,
    }
    (art / "manifest.json").write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n")
    for testset, group in results.items():
        print(f"{testset}: {group['test_count']} tests; matched verification hash; "
              f"median LCCC/GCC VM time {group['lccc_over_gcc_median_ratio_vm_only']:.3f}x")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
