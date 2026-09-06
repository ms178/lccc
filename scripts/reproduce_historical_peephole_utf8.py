#!/usr/bin/env python3
"""Compile and execute the exact pre-fix shared peephole helper in isolation.

The pre-fix tree immediately before 331f39c7 is not independently buildable
with the retained Rust toolchain because of unrelated split_ranges type errors.
This script therefore extracts the two self-contained functions that implement
the defect (`is_ident_char` and `replace_whole_word`) directly from that Git
blob, embeds them unchanged in a tiny Rust binary, and makes the legacy
byte-to-char recoding observable with a byte-exact assertion.

It is historical semantic evidence, not evidence of a released LCCC binary.
Publication provenance is handled separately by audit_peephole_utf8.py.
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
HISTORICAL_FIX = "331f39c7da2f12d2115372ffd2bd558fa91859f4"
SOURCE_PATH = "src/backend/peephole_common.rs"


def default_rustc() -> str:
    """Find rustc even when an automation shell did not source cargo's env."""
    discovered = shutil.which("rustc")
    if discovered:
        return discovered
    cargo_rustc = Path.home() / ".cargo" / "bin" / "rustc"
    if cargo_rustc.is_file() and os.access(cargo_rustc, os.X_OK):
        return str(cargo_rustc)
    return "rustc"


def manifest_toolchain(repo: Path) -> str:
    """Read the repository-selected channel without baking a Rust version in."""
    manifest = repo / "rust-toolchain.toml"
    if manifest.is_file():
        for line in manifest.read_text(encoding="utf-8").splitlines():
            key, sep, value = line.partition("=")
            if sep and key.strip() == "channel":
                channel = value.strip().strip('"').strip("'")
                if channel:
                    return channel
    return "stable"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def atomic_write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp.{os.getpid()}")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def git_show(repo: Path, spec: str) -> str:
    completed = subprocess.run(
        ["git", "show", spec],
        cwd=repo,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if completed.returncode:
        raise RuntimeError(f"git show {spec} failed: {completed.stderr.strip()}")
    return completed.stdout


def extract_function(source: str, name: str) -> str:
    """Extract one top-level Rust function with its original spelling intact."""
    markers = (f"pub(crate) fn {name}", f"fn {name}")
    starts = [source.find(marker) for marker in markers if source.find(marker) >= 0]
    if not starts:
        raise RuntimeError(f"function {name!r} was not found in historical source")
    start = min(starts)
    brace = source.find("{", start)
    if brace < 0:
        raise RuntimeError(f"function {name!r} has no opening brace")
    depth = 0
    for index in range(brace, len(source)):
        if source[index] == "{":
            depth += 1
        elif source[index] == "}":
            depth -= 1
            if depth == 0:
                return source[start : index + 1]
    raise RuntimeError(f"function {name!r} has no closing brace")


def run(command: list[str], timeout: int) -> dict[str, Any]:
    try:
        completed = subprocess.run(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            check=False,
        )
        return {
            "command": command,
            "returncode": completed.returncode,
            "stdout_tail": completed.stdout.decode("utf-8", "backslashreplace")[-2000:],
            "stderr_tail": completed.stderr.decode("utf-8", "backslashreplace")[-2000:],
        }
    except subprocess.TimeoutExpired as exc:
        return {
            "command": command,
            "returncode": 124,
            "stdout_tail": (exc.stdout or b"").decode("utf-8", "backslashreplace")[-2000:],
            "stderr_tail": (exc.stderr or b"").decode("utf-8", "backslashreplace")[-2000:],
        }
    except OSError as exc:
        return {"command": command, "returncode": 127, "stdout_tail": "", "stderr_tail": str(exc)}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=ROOT, help="repository root")
    parser.add_argument("--rustc", default=default_rustc(), help="Rust compiler")
    parser.add_argument("--timeout", type=int, default=120, help="compile/run timeout in seconds")
    parser.add_argument("--save-source", type=Path, help="save the generated standalone Rust source")
    parser.add_argument("--json", type=Path, help="write machine-readable evidence atomically")
    args = parser.parse_args()

    result: dict[str, Any] = {
        "reproducer": "historical shared peephole UTF-8 helper",
        "historical_fix": HISTORICAL_FIX,
        "historical_revision": f"{HISTORICAL_FIX}^",
        "source_path": SOURCE_PATH,
    }
    temporary: Path | None = None
    try:
        if args.timeout < 1:
            parser.error("--timeout must be positive")
        repo = args.repo.resolve()
        # `rustc` may be invoked from a temporary source directory; tell the
        # rustup proxy which manifest-selected channel to use explicitly so it
        # does not fall back to an old process-default toolchain.
        selected_toolchain = os.environ.setdefault("RUSTUP_TOOLCHAIN", manifest_toolchain(repo))
        result["rustup_toolchain"] = selected_toolchain
        historical = git_show(repo, f"{HISTORICAL_FIX}^:{SOURCE_PATH}")
        ident = extract_function(historical, "is_ident_char")
        replacement = extract_function(historical, "replace_whole_word")
        if "bytes[i] as char" not in replacement:
            raise RuntimeError("historical replacement function no longer matches the expected vulnerable shape")

        harness = f'''// Generated by scripts/reproduce_historical_peephole_utf8.py.
// Functions below are verbatim from {HISTORICAL_FIX}^:{SOURCE_PATH}.
#![allow(dead_code)]

{ident}

{replacement}

fn legacy_recode(text: &str) -> String {{
    text.as_bytes().iter().map(|&byte| byte as char).collect()
}}

fn main() {{
    let input = "add x1, x2  // café € 🦀";
    let expected = "add x9, x2  // café € 🦀";
    let observed = replace_whole_word(input, "x1", "x9");
    let legacy = legacy_recode(expected);
    assert_ne!(observed, expected, "historical helper unexpectedly preserved UTF-8");
    assert_eq!(observed, legacy, "historical helper did not follow byte-to-char recoding");
    println!("REPRODUCED legacy_byte_recode");
    println!("observed_hex={{:02x?}}", observed.as_bytes());
    println!("expected_hex={{:02x?}}", expected.as_bytes());
}}
'''
        harness_bytes = harness.encode("utf-8")
        result["historical_blob_sha256"] = sha256(historical.encode("utf-8"))
        result["extracted_replace_function_sha256"] = sha256(replacement.encode("utf-8"))
        result["generated_harness_sha256"] = sha256(harness_bytes)
        if args.save_source:
            args.save_source.parent.mkdir(parents=True, exist_ok=True)
            temporary_output = args.save_source.with_name(f".{args.save_source.name}.tmp.{os.getpid()}")
            temporary_output.write_bytes(harness_bytes)
            os.replace(temporary_output, args.save_source)
            result["saved_source"] = str(args.save_source)

        temporary = Path(tempfile.mkdtemp(prefix="lccc-ms09-historical-"))
        source = temporary / "historical.rs"
        binary = temporary / "historical-reproducer"
        source.write_bytes(harness_bytes)
        # Keep the historical source in its original Rust-2021 semantic mode;
        # this isolated reproducer demonstrates a past helper rather than
        # compiling the Rust-2024 LCCC crate itself.
        result["compile"] = run([args.rustc, "--edition=2021", "-O", str(source), "-o", str(binary)], args.timeout)
        if result["compile"]["returncode"] == 0:
            result["run"] = run([str(binary)], args.timeout)
        else:
            result["run"] = {"command": [str(binary)], "returncode": 127}
        result["status"] = (
            "PASS"
            if result["compile"]["returncode"] == 0
            and result["run"]["returncode"] == 0
            and "REPRODUCED legacy_byte_recode" in result["run"].get("stdout_tail", "")
            else "FAIL"
        )
    except (OSError, RuntimeError, subprocess.SubprocessError) as exc:
        result["status"] = "FAIL"
        result["error"] = f"{type(exc).__name__}: {exc}"
    finally:
        if temporary:
            shutil.rmtree(temporary, ignore_errors=True)

    if args.json:
        atomic_write_json(args.json, result)
    print(f"{result['status']}: historical shared helper reproduction")
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
