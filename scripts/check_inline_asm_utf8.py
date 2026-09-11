#!/usr/bin/env python3
"""Check byte-exact UTF-8 preservation through LCCC inline-assembly lowering.

The lexer intentionally represents C narrow string literal contents as one
Rust ``char`` per raw C byte.  That byte-carrier representation is correct for
ordinary C string data, but an inline-assembly template must be converted back
to UTF-8 text before it is appended to the textual assembler output.

This check compiles the MS-09 regression source to assembly, verifies an exact
UTF-8 marker inside its #APP/#NO_APP region, links/runs the program, and emits
hashes plus every command exit status as JSON.  ``--expect corrupt`` is for a
retained pre-fix baseline: it proves that the test is non-vacuous without
mistaking the baseline's intentional failure for an infrastructure failure.

Examples:
    scripts/check_inline_asm_utf8.py --lccc target/fastbuild/lccc \
      --json /tmp/inline-asm-utf8.json
    scripts/check_inline_asm_utf8.py --lccc /path/to/pre-fix-lccc \
      --expect corrupt --json /tmp/pre-fix.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SOURCE = ROOT / "tests" / "regression" / "peephole_utf8_inline_asm.c"
# Keep these literals synchronized with the markers in DEFAULT_SOURCE. They
# cover UTF-8 widths 2, 3, and 4; the second marker crosses C-string-literal
# boundaries, so it also checks that decoding happens after concatenation.
MARKER_TEXTS = (
    "# MS09 UTF-8: café € 🦀",
    "# MS09 split UTF-8: café € 🦀",
)
MARKERS = tuple(text.encode("utf-8") for text in MARKER_TEXTS)
# What the legacy path emits when it turns each original byte into U+00XX and
# then writes that Rust String as UTF-8.
LEGACY_MARKERS = tuple(marker.decode("latin-1").encode("utf-8") for marker in MARKERS)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run(command: list[str], timeout: int) -> dict[str, Any]:
    """Run a command without shell interpolation and retain bounded diagnostics."""
    try:
        proc = subprocess.run(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            check=False,
        )
        return {
            "command": command,
            "returncode": proc.returncode,
            "stdout_tail": proc.stdout.decode("utf-8", "backslashreplace")[-2000:],
            "stderr_tail": proc.stderr.decode("utf-8", "backslashreplace")[-2000:],
        }
    except subprocess.TimeoutExpired as exc:
        return {
            "command": command,
            "returncode": 124,
            "stdout_tail": (exc.stdout or b"").decode("utf-8", "backslashreplace")[-2000:],
            "stderr_tail": (exc.stderr or b"").decode("utf-8", "backslashreplace")[-2000:],
            "timeout_seconds": timeout,
        }
    except OSError as exc:
        return {
            "command": command,
            "returncode": 127,
            "stdout_tail": "",
            "stderr_tail": str(exc),
        }


def atomic_write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp.{os.getpid()}")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def app_region(assembly: bytes) -> bytes | None:
    """Return the first #APP..#NO_APP region, excluding the sentinel lines."""
    start = assembly.find(b"#APP\n")
    if start < 0:
        return None
    start += len(b"#APP\n")
    end = assembly.find(b"#NO_APP", start)
    if end < 0:
        return None
    return assembly[start:end]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", required=True, type=Path, help="LCCC executable to test")
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE, help="C regression source")
    parser.add_argument(
        "--expect",
        choices=("preserved", "corrupt"),
        default="preserved",
        help="expected marker condition; corrupt mode validates a known pre-fix binary",
    )
    parser.add_argument("--timeout", type=int, default=120, help="per-command timeout in seconds")
    parser.add_argument("--json", type=Path, help="write machine-readable result atomically")
    parser.add_argument("--save-asm", type=Path, help="copy generated assembly to this path")
    args = parser.parse_args()

    compiler = args.lccc.resolve()
    source = args.source.resolve()
    result: dict[str, Any] = {
        "checker": "check_inline_asm_utf8.py",
        "expect": args.expect,
        "compiler": str(compiler),
        "source": str(source),
        "markers_utf8_hex": [marker.hex() for marker in MARKERS],
        "legacy_markers_utf8_hex": [marker.hex() for marker in LEGACY_MARKERS],
    }

    if args.timeout < 1:
        parser.error("--timeout must be positive")
    if not compiler.is_file() or not os.access(compiler, os.X_OK):
        result.update({"status": "FAIL", "reason": "compiler is not executable"})
        if args.json:
            atomic_write_json(args.json, result)
        print(f"FAIL: compiler is not executable: {compiler}", file=sys.stderr)
        return 2
    if not source.is_file():
        result.update({"status": "FAIL", "reason": "source does not exist"})
        if args.json:
            atomic_write_json(args.json, result)
        print(f"FAIL: source does not exist: {source}", file=sys.stderr)
        return 2

    source_bytes = source.read_bytes()
    result["source_sha256"] = sha256(source_bytes)
    result["source_has_primary_exact_marker"] = MARKERS[0] in source_bytes
    result["source_has_split_utf8_fragments"] = all(
        fragment in source_bytes
        for fragment in (b"caf\\303\" \"\\251", b"\\342\\202\" \"\\254", b"\\360\\237\\246\" \"\\200")
    )

    tempdir = Path(tempfile.mkdtemp(prefix="lccc-inline-asm-utf8-"))
    asm = tempdir / "inline-asm.s"
    executable = tempdir / "inline-asm"
    try:
        result["compile_assembly"] = run(
            [str(compiler), "-O2", "-S", str(source), "-o", str(asm)], args.timeout
        )
        if result["compile_assembly"]["returncode"] == 0 and asm.is_file():
            assembly = asm.read_bytes()
            region = app_region(assembly)
            result.update(
                {
                    "assembly_sha256": sha256(assembly),
                    "assembly_valid_utf8": _is_valid_utf8(assembly),
                    "has_app_region": region is not None,
                    "exact_markers_in_assembly": [marker in assembly for marker in MARKERS],
                    "legacy_markers_in_assembly": [marker in assembly for marker in LEGACY_MARKERS],
                    "exact_markers_in_app_region": [
                        region is not None and marker in region for marker in MARKERS
                    ],
                    "legacy_markers_in_app_region": [
                        region is not None and marker in region for marker in LEGACY_MARKERS
                    ],
                }
            )
            if args.save_asm:
                args.save_asm.parent.mkdir(parents=True, exist_ok=True)
                temporary = args.save_asm.with_name(f".{args.save_asm.name}.tmp.{os.getpid()}")
                temporary.write_bytes(assembly)
                os.replace(temporary, args.save_asm)
                result["saved_assembly"] = str(args.save_asm)
        else:
            result.update(
                {
                    "assembly_sha256": None,
                    "assembly_valid_utf8": False,
                    "has_app_region": False,
                    "exact_markers_in_assembly": [False] * len(MARKERS),
                    "legacy_markers_in_assembly": [False] * len(MARKERS),
                    "exact_markers_in_app_region": [False] * len(MARKERS),
                    "legacy_markers_in_app_region": [False] * len(MARKERS),
                }
            )

        result["compile_executable"] = run(
            [str(compiler), "-O2", str(source), "-o", str(executable)], args.timeout
        )
        if result["compile_executable"]["returncode"] == 0 and executable.is_file():
            result["executable_sha256"] = sha256(executable.read_bytes())
            result["run_executable"] = run([str(executable)], args.timeout)
        else:
            result["executable_sha256"] = None
            result["run_executable"] = {"command": [str(executable)], "returncode": 127}

        infrastructure_ok = (
            result["source_has_primary_exact_marker"]
            and result["source_has_split_utf8_fragments"]
            and result["compile_assembly"]["returncode"] == 0
            and result["compile_executable"]["returncode"] == 0
            and result["run_executable"]["returncode"] == 0
            and result["assembly_valid_utf8"]
            and result["has_app_region"]
        )
        if args.expect == "preserved":
            marker_ok = (
                all(result["exact_markers_in_app_region"])
                and not any(result["legacy_markers_in_app_region"])
            )
        else:
            marker_ok = (
                all(result["legacy_markers_in_app_region"])
                and not any(result["exact_markers_in_app_region"])
            )
        result["marker_condition_ok"] = marker_ok
        result["infrastructure_ok"] = infrastructure_ok
        result["status"] = "PASS" if infrastructure_ok and marker_ok else "FAIL"
    finally:
        shutil.rmtree(tempdir, ignore_errors=True)

    if args.json:
        atomic_write_json(args.json, result)
    summary = (
        f"{result['status']}: expect={args.expect} "
        f"exact={result.get('exact_markers_in_app_region', [])} "
        f"legacy={result.get('legacy_markers_in_app_region', [])} "
        f"run_rc={result.get('run_executable', {}).get('returncode', 127)}"
    )
    print(summary)
    return 0 if result["status"] == "PASS" else 1


def _is_valid_utf8(data: bytes) -> bool:
    try:
        data.decode("utf-8")
        return True
    except UnicodeDecodeError:
        return False


if __name__ == "__main__":
    raise SystemExit(main())
