#!/usr/bin/env python3
"""Causal comparison of built PR827, pre-range PR826, and the current compiler.

Snapshots are extracted and built by CI, never switched into the working branch.
All operands/recipes come from the current strict contract and corpus registry.
Baseline failures are observations, not successes of the candidate.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import asmdiff

PGO = runpy.run_path(str(ROOT / "tests/regression/check_pgo_io.py"), run_name="compare_pgo")
BENCH = runpy.run_path(str(ROOT / "tests/benchmark/run_benchmarks.py"), run_name="compare_corpus")


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def invoke(command, **kwargs):
    return subprocess.run([str(x) for x in command], capture_output=True, text=True,
                          timeout=kwargs.pop("timeout", 180), cwd=ROOT, **kwargs)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--agent-b", required=True, type=Path, help="directory containing lccc and lccc-riscv")
    ap.add_argument("--prior-ours", required=True, type=Path)
    ap.add_argument("--candidate", required=True, type=Path)
    ap.add_argument("--cc", default="gcc-16")
    ap.add_argument("--output", required=True, type=Path)
    args = ap.parse_args()
    if not __debug__:
        ap.error("optimized Python disables test assertions")
    args.output.mkdir(parents=True, exist_ok=True)
    versions = {"agent-b": args.agent_b.resolve(), "prior-ours": args.prior_ours.resolve(),
                "candidate": args.candidate.resolve()}
    summaries, objects, codegen, probes = {}, {}, [], []
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("LCCC_PGO_", "PGO_IO_", "DRIVER_"))
           and k not in ("LCCC_PROFILE_FILE", "LLVM_PROFILE_FILE", "LD_PRELOAD")}
    failures = []
    try:
        linker_hashes = []
        for label, directory in versions.items():
            for binary in (directory / "lccc", directory / "lccc-riscv"):
                if not binary.is_file() or not os.access(binary, os.X_OK):
                    raise RuntimeError(f"{label}: missing/non-executable compiler {binary}")
            linker_hashes.append(digest(directory / "lccc-riscv"))
        if len(set(linker_hashes)) != 3:
            raise RuntimeError("causal arms reused identical linker binaries; reject stale/cross-source build fingerprints")
        with tempfile.TemporaryDirectory(prefix="lccc-pr827-compare-") as tmp:
            root = Path(tmp)
            strict, diagnostic = root / "strict.so", root / "diagnostic.so"
            for output, extra in ((strict, []), (diagnostic, ["-DPGO_IO_DIAGNOSTIC_ONLY"])):
                command = [args.cc, "-shared", "-fPIC", "-O1", "-Wall", "-Wextra", "-Werror", *extra,
                           ROOT / "tests/regression/fixtures/pgo_io_interpose.c", "-ldl", "-o", output]
                p = invoke(command)
                if p.returncode:
                    raise RuntimeError(f"preload compilation failed: {p.stderr}")
            for label, directory in versions.items():
                compiler, linker = directory / "lccc", directory / "lccc-riscv"
                for binary in (compiler, linker):
                    if not binary.is_file() or not os.access(binary, os.X_OK):
                        raise RuntimeError(f"{label}: missing/non-executable compiler {binary}")
                report = args.output / f"{label}-relocations.json"
                p = invoke([sys.executable, ROOT / "tests/linker/check_riscv_relocation_contract.py",
                            "--lccc", linker, "--json", report], timeout=900, env=env)
                (args.output / f"{label}-relocations.log").write_text(p.stdout + p.stderr)
                if p.returncode not in (0, 1) or not report.is_file():
                    raise RuntimeError(f"{label}: contract infrastructure failed: {p.stderr}")
                result = json.loads(report.read_text())
                records = result["records"]
                groups = {}
                for prefix in ("valid-range-", "bad-range-", "valid-uleb-", "bad-uleb-", "valid-none-empty"):
                    rows = [row for row in records if row["case"].startswith(prefix)]
                    groups[prefix] = {"total": len(rows), "passed": sum(row["passed"] for row in rows)}
                summaries[label] = {"passed": result["passed"], "failed": result["failed"],
                                    "total": len(records), "groups": groups,
                                    "linker_sha256": digest(linker), "compiler_sha256": digest(compiler),
                                    "report_sha256": digest(report)}
                if label == "candidate" and result["failed"]:
                    failures.append("candidate relocation contract failed")
                # Compile/run actual generated PGO code, not a C reconstruction.
                source, executable = root / "runtime.c", root / f"runtime-{label}"
                source.write_text(PGO["source"](96))
                golden = root / f"golden-{label}.profraw"
                p = invoke([compiler, "-O2", "-fprofile-generate=" + str(golden), source, "-o", executable], env=env)
                if p.returncode:
                    raise RuntimeError(f"{label}: generated runtime compilation failed: {p.stderr}")
                expected_stdout = f"{96 - 96 * 95 // 2}\n"
                for name, preload, fault in (("close-on-exec", strict, None),
                                             ("ferror-zero", diagnostic, "ferror-zero"),
                                             ("ferror-stale", diagnostic, "ferror-stale")):
                    profile = root / f"probe-{label}-{name}.profraw"
                    runtime_env = dict(env, LD_PRELOAD=str(preload), LCCC_PROFILE_FILE=str(profile))
                    if fault:
                        runtime_env["PGO_IO_FAULT"] = fault
                    p = invoke([executable], env=runtime_env)
                    passed = p.returncode == 23 and p.stdout == expected_stdout
                    if fault:
                        passed = passed and p.stderr == "lccc: PGO: profile write failed\n"
                    else:
                        passed = passed and not p.stderr
                    probes.append({"version": label, "probe": name, "passed": passed,
                                   "returncode": p.returncode, "stdout": p.stdout, "stderr": p.stderr})
                    if label == "candidate" and not passed:
                        failures.append(f"candidate runtime probe {name} failed")
                # Real syscall and memory tools; status 23 remains the application status.
                trace = args.output / f"{label}-strace.txt"
                p = invoke(["strace", "-o", trace, "-e", "trace=openat,flock,close,write", executable], env=env)
                (args.output / f"{label}-strace-stderr.txt").write_text(p.stderr)
                if p.returncode != 23 or p.stdout != expected_stdout:
                    failures.append(f"{label}: syscall trace execution failed")
                p = invoke(["valgrind", "--error-exitcode=99", "--leak-check=full", executable], env=env)
                (args.output / f"{label}-valgrind.txt").write_text(p.stderr)
                if p.returncode != 23 or p.stdout != expected_stdout or "ERROR SUMMARY: 0 errors" not in p.stderr:
                    failures.append(f"{label}: memory-tool execution failed")
                # Reuse registered recipes, including scaled workload macro flags.
                for opt in ("-O2", "-O3"):
                    for benchmark in BENCH["BENCHMARKS"]:
                        source = BENCH["source_for"](benchmark)
                        if source is None:
                            raise RuntimeError(f"missing registered source {benchmark.name}")
                        output = root / f"{label}-{opt}-{benchmark.name}.o"
                        spec = BENCH["CompilerSpec"](label, label, str(compiler), (opt, "-c"))
                        result = BENCH["compile_one"](spec, benchmark, source, output, None)
                        if not result["ok"]:
                            raise RuntimeError(f"{label}/{opt}/{benchmark.name}: {result}")
                        image = asmdiff.read_elf(output)
                        # This is exact object identity, not a perf or correctness claim.
                        objects.setdefault((opt, benchmark.name), {})[label] = digest(output)
                        codegen.append({"version": label, "opt": opt, "workload": benchmark.name,
                                        "source_sha256": result["source_sha256"], "command": result["command"],
                                        "object_sha256": digest(output),
                                        "section_bytes": {name: len(data) for name, data in image.content.items()},
                                        "relocations": sum(len(rows) for rows in image.relocs.values())})
            identities = [{"opt": opt, "workload": name, "sha256": hashes,
                           "identical": len(hashes) == 3 and len(set(hashes.values())) == 1}
                          for (opt, name), hashes in objects.items()]
            if len(identities) != len(BENCH["BENCHMARKS"]) * 2 or not all(row["identical"] for row in identities):
                failures.append("registered non-PGO codegen object identity differs; investigate before claiming parity")
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired, AssertionError) as error:
        failures.append(f"comparison infrastructure: {type(error).__name__}: {error}")
        identities = []
    result = {"versions": summaries, "runtime_probes": probes, "codegen": codegen,
              "object_identities": identities, "timing_claim": False, "failures": failures}
    (args.output / "comparison.json").write_text(json.dumps(result, indent=2) + "\n")
    compact = {"versions": summaries, "runtime_probes": probes,
               "identical_object_pairs": sum(row["identical"] for row in identities),
               "object_pairs": len(identities), "timing_claim": False, "failures": failures}
    notice = json.dumps(compact, sort_keys=True).replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print("::notice title=PR827 causal comparison::" + notice)
    for failure in failures:
        print("::error title=PR827 comparison::" + failure.replace("%", "%25").replace("\n", "%0A"))
    return int(bool(failures))


if __name__ == "__main__":
    raise SystemExit(main())
