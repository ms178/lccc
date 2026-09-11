#!/usr/bin/env python3
"""Require byte-identical assembly from two LCCC binaries for a C corpus.

This is deliberately stricter than an execution oracle.  It is for a pure
backend refactor whose documented contract is to leave the default generated
code unchanged: each source is compiled twice under the same codegen flags and
any changed byte, compilation failure, or one-sided failure is an error.

``CCC_*`` and ``LCCC_*`` variables inherited from the caller are removed before
both runs.  That makes a default-policy comparison reproducible even from a
shell that was previously used for kill-switch bisection.  Per-source ``.env``
files are intentionally not loaded for the same reason.  Adjacent ``.flags``
files are loaded; an absent or empty flags file means ``-O2``, matching the
regression runner.  LCCC's PGO-generation mode embeds the compiler process ID in
its otherwise equivalent generated profile-file/symbol name.  That documented
nondeterminism is canonicalized only for ``-fprofile-generate`` comparisons;
raw assembly is preserved and separately hashed in the report.

Examples:
    scripts/check_codegen_refactor_identity.py \\
      --before /path/to/lccc-before --after target/fastbuild/lccc \\
      --corpus tests/benchmark/kernel_corpus --out /tmp/kernel-identity

    scripts/check_codegen_refactor_identity.py \\
      --before /path/to/lccc-before --after target/fastbuild/lccc \\
      --corpus tests/regression --extra=-I"$(gcc -print-file-name=include)" \\
      --jobs 2 --out /tmp/regression-identity

    # Retain exactly one documented bisection knob for a compatibility smoke.
    scripts/check_codegen_refactor_identity.py \\
      --before /path/to/lccc-before --after target/fastbuild/lccc \\
      --corpus tests/benchmark/kernel_corpus --env CCC_NO_SEGMENT_SCAN=1 \\
      --out /tmp/segment-scan-identity
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any


@dataclass
class Result:
    source: str
    flags: list[str]
    before_rc: int
    after_rc: int
    before_sha256: str | None
    after_sha256: str | None
    before_canonical_sha256: str | None
    after_canonical_sha256: str | None
    verdict: str
    detail: str = ""


# PGO generate mode intentionally gives its .profraw helper a process-ID
# suffix.  It appears both as an emitted string and as a hidden helper symbol.
# Restrict canonicalization to these two generated forms and only when the
# source was compiled with -fprofile-generate, so an ordinary numeric codegen
# difference is never hidden by this identity gate.
_PGO_DUMP_PID = re.compile(rb"(__lccc_pgo_dump_[0-9a-f]+_)[0-9]+")
_PGO_RAW_PID = re.compile(rb"(lccc-(?:[A-Za-z0-9_]+-)+)[0-9]+(\.profraw)")
# The x86 emitter materializes PGO paths as decimal .byte values rather than
# an .asciz string.  Match only the ASCII ``-<pid>.profraw\0`` suffix.
_BYTE_SEP = rb"(?:,\s*(?:\.byte\s*)?|\s+\.byte\s*)"
_PGO_RAW_PID_BYTES = re.compile(
    rb"45(?:" + _BYTE_SEP + rb"(?:4[89]|5[0-7])){1,10}"
    + _BYTE_SEP + rb"46" + _BYTE_SEP + rb"112" + _BYTE_SEP + rb"114"
    + _BYTE_SEP + rb"111" + _BYTE_SEP + rb"102" + _BYTE_SEP + rb"114"
    + _BYTE_SEP + rb"97" + _BYTE_SEP + rb"119" + _BYTE_SEP + rb"0"
)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def assembly_bytes(path: Path, flags: list[str]) -> bytes:
    data = path.read_bytes()
    if any(flag.startswith("-fprofile-generate") for flag in flags):
        data = _PGO_DUMP_PID.sub(rb"\1<PID>", data)
        data = _PGO_RAW_PID.sub(rb"\1<PID>\2", data)
        data = _PGO_RAW_PID_BYTES.sub(
            b"45, <PID>, 46, 112, 114, 111, 102, 114, 97, 119, 0", data
        )
    return data


def first_difference(left: bytes, right: bytes) -> str:
    """Return a compact byte-level diagnosis without printing whole assembly."""
    for offset, (x, y) in enumerate(zip(left, right)):
        if x != y:
            return f"first byte differs at offset {offset}: {x:#04x} != {y:#04x}"
    return f"file lengths differ at offset {min(len(left), len(right))}"


def normalized_env() -> tuple[dict[str, str], list[str]]:
    env = dict(os.environ)
    removed = sorted(key for key in env if key.startswith(("CCC_", "LCCC_")))
    for key in removed:
        del env[key]
    return env, removed


def flags_for(source: Path, default_flags: str, profile_dir: Path) -> list[str]:
    sidecar = source.with_suffix(".flags")
    raw = sidecar.read_text().strip() if sidecar.exists() else default_flags
    if not raw:
        raw = default_flags
    # PGO regression flags use this marker.  Compilation to assembly does not
    # consume a profile, but preserving one stable path prevents a different
    # synthetic location from turning a codegen identity run into a false diff.
    raw = raw.replace("@PROFDIR@", str(profile_dir))
    return shlex.split(raw)


def run_compiler(
    compiler: Path,
    source: Path,
    flags: list[str],
    output: Path,
    extra: list[str],
    env: dict[str, str],
    timeout: int,
) -> tuple[int, str]:
    output.parent.mkdir(parents=True, exist_ok=True)
    command = [str(compiler), *extra, *flags, str(source), "-S", "-o", str(output)]
    try:
        proc = subprocess.run(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return 124, f"timeout after {timeout}s"
    except OSError as exc:
        return 127, str(exc)
    # Only retain the tail. Full diagnostic logs from a large corpus are not
    # useful in JSON and can obscure the source responsible for a failure.
    return proc.returncode, proc.stderr[-2000:]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True, help="baseline LCCC executable")
    parser.add_argument("--after", type=Path, required=True, help="candidate LCCC executable")
    parser.add_argument(
        "--corpus",
        type=Path,
        action="append",
        required=True,
        help="directory recursively containing .c sources; may be supplied more than once",
    )
    parser.add_argument("--out", type=Path, required=True, help="directory for assembly and report")
    parser.add_argument("--extra", action="append", default=[], help="extra compiler flag; repeatable")
    parser.add_argument(
        "--env",
        action="append",
        default=[],
        metavar="KEY=VALUE",
        help="explicit variable applied after the default-policy scrub; repeatable",
    )
    parser.add_argument("--default-flags", default="-O2", help="flags for a source without a .flags sidecar")
    parser.add_argument("--jobs", type=int, default=1, help="parallel compiler invocations (default: 1)")
    parser.add_argument("--timeout", type=int, default=120, help="per compiler invocation timeout in seconds")
    args = parser.parse_args()

    if args.jobs < 1:
        parser.error("--jobs must be positive")
    for compiler, label in ((args.before, "--before"), (args.after, "--after")):
        if not compiler.is_file() or not os.access(compiler, os.X_OK):
            parser.error(f"{label} is not an executable file: {compiler}")

    corpora = [path.resolve() for path in args.corpus]
    for corpus in corpora:
        if not corpus.is_dir():
            parser.error(f"--corpus is not a directory: {corpus}")
    sources: list[tuple[Path, Path]] = []
    for corpus in corpora:
        sources.extend((corpus, source) for source in sorted(corpus.rglob("*.c")))
    if not sources:
        parser.error("no C sources found")

    output = args.out.resolve()
    if output.exists():
        shutil.rmtree(output)
    (output / "before").mkdir(parents=True)
    (output / "after").mkdir()
    profile_dir = output / "profile-marker"
    profile_dir.mkdir()
    env, removed = normalized_env()
    requested_env: dict[str, str] = {}
    for assignment in args.env:
        key, separator, value = assignment.partition("=")
        if not separator or not key:
            parser.error(f"--env must be KEY=VALUE, got {assignment!r}")
        requested_env[key] = value
    env.update(requested_env)
    began = time.monotonic()

    def compare(item: tuple[Path, Path]) -> Result:
        corpus, source = item
        # A corpus directory name scopes the path when multiple corpora have
        # equally named files (common for small extracted kernels).
        rel = Path(corpus.name) / source.relative_to(corpus)
        flags = flags_for(source, args.default_flags, profile_dir)
        before = output / "before" / rel.with_suffix(".s")
        after = output / "after" / rel.with_suffix(".s")
        before_rc, before_err = run_compiler(
            args.before, source, flags, before, args.extra, env, args.timeout
        )
        after_rc, after_err = run_compiler(
            args.after, source, flags, after, args.extra, env, args.timeout
        )
        name = str(rel)
        if before_rc != 0 or after_rc != 0:
            detail = (
                f"before rc={before_rc}: {before_err.strip()[-500:]} | "
                f"after rc={after_rc}: {after_err.strip()[-500:]}"
            )
            return Result(
                name, flags, before_rc, after_rc, None, None, None, None, "COMPILE-FAIL", detail
            )
        raw_before = before.read_bytes()
        raw_after = after.read_bytes()
        left = sha256_bytes(raw_before)
        right = sha256_bytes(raw_after)
        canonical_before = assembly_bytes(before, flags)
        canonical_after = assembly_bytes(after, flags)
        canonical_left = sha256_bytes(canonical_before)
        canonical_right = sha256_bytes(canonical_after)
        if left == right:
            return Result(
                name,
                flags,
                before_rc,
                after_rc,
                left,
                right,
                canonical_left,
                canonical_right,
                "IDENTICAL",
            )
        if canonical_left == canonical_right:
            return Result(
                name,
                flags,
                before_rc,
                after_rc,
                left,
                right,
                canonical_left,
                canonical_right,
                "IDENTICAL-PGO-PID",
                "raw PGO output differs only in the compiler process-ID suffix",
            )
        return Result(
            name,
            flags,
            before_rc,
            after_rc,
            left,
            right,
            canonical_left,
            canonical_right,
            "DIFFERENT",
            first_difference(canonical_before, canonical_after),
        )

    results: list[Result] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(compare, source) for source in sources]
        for future in concurrent.futures.as_completed(futures):
            result = future.result()
            results.append(result)
            if result.verdict not in {"IDENTICAL", "IDENTICAL-PGO-PID"}:
                print(f"{result.verdict:18} {result.source}: {result.detail}")
    results.sort(key=lambda result: result.source)

    summary: dict[str, Any] = {
        "before": str(args.before.resolve()),
        "after": str(args.after.resolve()),
        "corpora": [str(path) for path in corpora],
        "default_flags": args.default_flags,
        "extra": args.extra,
        "removed_environment": removed,
        "explicit_environment": requested_env,
        "elapsed_seconds": round(time.monotonic() - began, 3),
        "total": len(results),
        "identical": sum(result.verdict == "IDENTICAL" for result in results),
        "identical_pgo_pid": sum(result.verdict == "IDENTICAL-PGO-PID" for result in results),
        "different": sum(result.verdict == "DIFFERENT" for result in results),
        "compile_fail": sum(result.verdict == "COMPILE-FAIL" for result in results),
        "results": [asdict(result) for result in results],
    }
    report = output / "report.json"
    report.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")

    print(
        "assembly identity: "
        f"IDENTICAL={summary['identical']} PGO_PID={summary['identical_pgo_pid']} "
        f"DIFFERENT={summary['different']} COMPILE_FAIL={summary['compile_fail']} "
        f"TOTAL={summary['total']} "
        f"({summary['elapsed_seconds']:.2f}s)"
    )
    print(f"report: {report}")
    return 0 if summary["identical"] + summary["identical_pgo_pid"] == summary["total"] else 1


if __name__ == "__main__":
    sys.exit(main())
