#!/usr/bin/env python3
"""Optional Clang 23 whole-SQLite reference against a validated GCC/LCCC run.

Reuse the SAME pinned and patched SQLite source, recipe flags and SQL corpus;
perform one verified upstream speedtest run per set. This is a correctness gate,
not a precise compiler-performance benchmark (VM times are reported separately).
Requires a retained baseline work directory (KEEP_WORK=1 in run.sh).
"""
from __future__ import annotations

import argparse
import json
import os
import pathlib
import shlex
import shutil
import subprocess

from summarize import parse_run


HERE = pathlib.Path(__file__).resolve().parent


def logged(command: list[str], path: pathlib.Path, *, cwd: pathlib.Path,
           env: dict[str, str] | None = None, stdin: bytes | None = None,
           timeout: int = 1200) -> None:
    with path.open("wb") as log:
        result = subprocess.run(command, cwd=cwd, env=env, input=stdin,
                                stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"command exited {result.returncode}: {command!r} (see {path})")


def no_warnings(paths: list[pathlib.Path]) -> None:
    for path in paths:
        for number, line in enumerate(path.read_text(errors="replace").splitlines(), 1):
            if "warning" in line.lower():
                raise ValueError(f"diagnostic in {path}:{number}: {line}")


def run_speedtest(command: list[str], log: pathlib.Path, *, cwd: pathlib.Path) -> None:
    # Build logs deliberately combine stdout/stderr for diagnostics, but a
    # speedtest's verification hash lives on stdout: joining streams let a
    # runtime warning slip into a "PASS" report. Preserve each stream and
    # require silence just as the GCC/LCCC and paired workload gates do.
    result = subprocess.run(command, cwd=cwd, capture_output=True, timeout=180)
    log.write_bytes(result.stdout)
    err = log.with_suffix(".err")
    err.write_bytes(result.stderr)
    if result.returncode or result.stderr:
        raise RuntimeError(f"speedtest exited {result.returncode} or wrote stderr: "
                           f"{log} (stderr: {err})")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--baseline", type=pathlib.Path, required=True)
    ap.add_argument("--out", type=pathlib.Path, required=True)
    ap.add_argument("--clang", default=os.environ.get("CLANG", "clang-23"))
    args = ap.parse_args()
    baseline = args.baseline.resolve()
    output = args.out.resolve()
    if baseline == output:
        ap.error("output directory must differ from the validated baseline")
    output.mkdir(parents=True, exist_ok=True)
    # Invalidate BEFORE inspecting inputs, checking Clang availability, or
    # rejecting a retained build tree: failed reruns cannot leave an old PASS.
    (output / "manifest.json").unlink(missing_ok=True)
    if not shutil.which(args.clang):
        ap.error("an installed Clang is required")
    manifest = json.loads((baseline / "manifest.json").read_text())
    if manifest.get("status") != "PASS" or manifest.get("version") != "3.54.1":
        raise ValueError("expected validated SQLite 3.54.1 baseline")
    work = pathlib.Path((baseline / "work-path.txt").read_text().strip())
    source = work / "sqlite-version-3.54.1"
    if (source / "manifest.uuid").read_text().strip() != manifest["fossil_manifest_uuid"]:
        raise ValueError("retained source identity differs from validated baseline")
    build = work / "build-clang"
    if build.exists():
        raise ValueError(f"Clang build tree already exists (refuse stale objects): {build}")
    build.mkdir()
    clang = str(pathlib.Path(shutil.which(args.clang) or args.clang).resolve())
    cc_env = dict(os.environ, CC=clang, CFLAGS=manifest["cflags"],
                  CPPFLAGS=manifest["cppflags"], LDFLAGS="-Wl,-z,relro -Wl,-z,now")
    configs = ["--disable-shared", "--disable-tcl", "--disable-readline", "--fts5",
               "--rtree", "--column-metadata", "--session"]
    configure, build_log, speedbuild = [output / f"{name}.log" for name in
                                       ("configure", "build", "speedtest-build")]
    print(f"Clang {subprocess.check_output([clang, '--version'], text=True).splitlines()[0]}",
          flush=True)
    logged([str(source / "configure"), *configs], configure, cwd=build, env=cc_env)
    logged(["make", "-j2", "sqlite3", "libsqlite3.a"], build_log, cwd=build,
           env=cc_env)
    logged([clang, *shlex.split(manifest["cflags"]), *shlex.split(manifest["cppflags"]),
            "-DSQLITE_ENABLE_RTREE", "-I.", f"-I{source}",
            str(source / "test/speedtest1.c"), "libsqlite3.a", "-o", "speedtest1",
            "-ldl", "-lpthread", "-lm"], speedbuild, cwd=build)
    no_warnings([configure, build_log, speedbuild])
    for case in manifest["sql_cases"]:
        result = subprocess.run([str(build / "sqlite3"), "-batch", "-bail", ":memory:"],
                                input=(HERE / f"{case}.sql").read_bytes(),
                                capture_output=True, timeout=120)
        output_file = output / f"sql-{case}-clang.txt"
        output_file.write_bytes(result.stdout)
        (output / f"sql-{case}-clang.err").write_bytes(result.stderr)
        if (result.returncode or result.stderr or
                result.stdout != (baseline / f"sql-{case}-gcc.txt").read_bytes()):
            raise ValueError(f"SQL/Clang differs from GCC: {output_file}")
        print(f"{case}: SQL and integrity match GCC", flush=True)
    results = {}
    for testset, expected in manifest["speedtest1"].items():
        db = build / f"clang-{testset}.db"
        text = output / f"bench-{testset}-clang.txt"
        run_speedtest([str(build / "speedtest1"), "--size", str(manifest["speedtest_size"]),
                       "--nosync", "--testset", testset, "--verify", str(db)], text,
                      cwd=build)
        run = parse_run(text, expected_set=testset)
        if (run["verification_hash"] != expected["verification_hash"] or
                run["verification_bytes"] != expected["verified_bytes"] or
                run["tests"] != [tuple(row) for row in expected["runs"]["gcc"][0]["tests"]] or
                len(run["tests"]) != expected["test_count"]):
            raise ValueError(f"Clang upstream speedtest mismatch: {text}")
        integrity = subprocess.run([str(build / "sqlite3"), "-batch", "-bail", str(db),
                                    "PRAGMA integrity_check;"], capture_output=True, timeout=120)
        if integrity.returncode or integrity.stdout.strip() != b"ok" or integrity.stderr:
            raise ValueError(f"Clang speedtest integrity failed: {db}")
        db.unlink()
        results[testset] = {
            "verification_hash": run["verification_hash"],
            "test_count": len(run["tests"]), "clang_vm_seconds": run["total_seconds"],
            "gcc_median_vm_seconds": expected["gcc_median_seconds"],
            "clang_over_gcc_vm_ratio": (run["total_seconds"] / expected["gcc_median_seconds"]),
        }
        print(f"{testset}: Clang/GCC hashes and test identities match; "
              f"VM ratio {results[testset]['clang_over_gcc_vm_ratio']:.3f}", flush=True)
    (output / "manifest.json").write_text(json.dumps({
        "schema": 1, "status": "PASS", "source_zip_sha256": manifest["source_zip_sha256"],
        "baseline": str(baseline), "clang": clang,
        "clang_version": subprocess.check_output([clang, "--version"], text=True).splitlines()[0],
        "cflags": manifest["cflags"], "cppflags": manifest["cppflags"],
        "build": str(build), "warnings": 0,
        "timing_caveat": "single-run VM screen only; not native CPU performance evidence",
        "speedtest1": results,
    }, indent=2) + "\n")
    print(f"PASS: Clang/GCC SQLite 3.54.1 SQL, six verified speedtest sets, integrity, warnings ({output})")


if __name__ == "__main__":
    main()
