#!/usr/bin/env python3
"""Reproducible Compiler Explorer oracle for LCCC code-generation research.

The module API is intentionally small so other tools can reuse it::

    compile_on_godbolt(compiler_id, source, flags, intel=False)
    list_compilers("c")

CLI examples::

    scripts/godbolt.py list --filter 'gcc 16.2|icx|icc'
    scripts/godbolt.py compile gcc16.2 kernel.c --flags '-O3 -march=raptorlake'
    scripts/godbolt.py compare kernel.c --local target/fastbuild/lccc \
        --oracles gcc16.2,clang,icc,icx --function hot_loop \
        --artifact-dir results/hot-loop --json results/hot-loop/manifest.json

Aliases are pinned where a fixed release exists. ``icx`` deliberately resolves
through Compiler Explorer's ``cicxlatest`` channel; every comparison manifest
records the resolved id/name so a future rerun cannot silently claim it used the
same compiler.  Assembly is requested in AT&T syntax to match LCCC and permit
operand-side load/store analysis in codegen_scoreboard.py.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shlex
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

API = os.environ.get("GODBOLT_API", "https://godbolt.org/api").rstrip("/")
USER_AGENT = "lccc-codegen-research/2 (+https://github.com/ms178/lccc)"
CACHE = Path(os.environ.get(
    "GODBOLT_CACHE",
    str(Path(__file__).resolve().parent.parent / ".godbolt-cache"),
))

# Required competition set. Keep these ids reviewable and deterministic.
# ICC Classic ended at 2021.10.0; ICX remains a moving channel and is therefore
# identified by resolved API metadata in every artifact manifest.
COMPILER_ALIASES: dict[str, str] = {
    "gcc": "cg162",
    "gcc16.2": "cg162",
    "clang": "cclang2210",
    "clang22.1": "cclang2210",
    "icc": "cicc2021100",
    "icc2021.10": "cicc2021100",
    "icx": "cicxlatest",
    "icx-latest": "cicxlatest",
}
DEFAULT_ORACLES = ("gcc16.2", "clang", "icc", "icx")


class GodboltError(RuntimeError):
    """Compiler Explorer request or compilation failure."""


def _request_json(url: str, *, body: dict[str, Any] | None = None,
                  timeout: int = 120, retries: int = 3) -> Any:
    data = json.dumps(body).encode() if body is not None else None
    headers = {"Accept": "application/json", "User-Agent": USER_AGENT}
    if body is not None:
        headers["Content-Type"] = "application/json"
    last: Exception | None = None
    for attempt in range(retries):
        try:
            req = urllib.request.Request(url, data=data, headers=headers)
            with urllib.request.urlopen(req, timeout=timeout) as response:
                return json.load(response)
        except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError,
                json.JSONDecodeError) as error:
            last = error
            # 4xx other than rate limiting is deterministic; retrying only
            # wastes API capacity and obscures the useful response body.
            if isinstance(error, urllib.error.HTTPError) and error.code < 500 \
                    and error.code != 429:
                detail = error.read().decode("utf-8", "replace")[:2000]
                raise GodboltError(f"HTTP {error.code}: {detail}") from error
            if attempt + 1 < retries:
                time.sleep(0.5 * (2 ** attempt))
    raise GodboltError(f"Compiler Explorer request failed after {retries} attempts: {last}")


def list_compilers(language: str = "c", *, refresh: bool = False) -> list[dict[str, Any]]:
    """Return CE compiler metadata, cached briefly for reproducible alias lookup."""
    cache = CACHE / f"compilers-{language}.json"
    CACHE.mkdir(parents=True, exist_ok=True)
    if cache.exists() and not refresh and time.time() - cache.stat().st_mtime < 86400:
        try:
            value = json.loads(cache.read_text())
            if isinstance(value, list):
                return value
        except (OSError, json.JSONDecodeError):
            pass
    value = _request_json(f"{API}/compilers/{language}", timeout=60)
    if not isinstance(value, list):
        raise GodboltError("unexpected compiler-list response")
    tmp = cache.with_suffix(f".tmp.{os.getpid()}")
    tmp.write_text(json.dumps(value, indent=1))
    tmp.replace(cache)
    return value


def resolve_compiler(name: str) -> str:
    """Resolve a reviewable alias or pass through an explicit CE compiler id."""
    return COMPILER_ALIASES.get(name.lower(), name)


def compiler_metadata(compiler_id: str, *, compilers: list[dict[str, Any]] | None = None) -> dict[str, Any]:
    cid = resolve_compiler(compiler_id)
    entries = compilers if compilers is not None else list_compilers("c")
    found = next((item for item in entries if item.get("id") == cid), None)
    if found is None:
        return {"id": cid, "name": "unknown", "semver": "unknown"}
    return {
        "id": cid,
        "name": found.get("name", "unknown"),
        "semver": found.get("semver", "unknown"),
        "compilerType": found.get("compilerType", "unknown"),
    }


def compile_on_godbolt(compiler_id: str, source: str, flags: str,
                        *, intel: bool = False, timeout: int = 120) -> dict[str, Any] | None:
    """Compile C source on CE and return its JSON result.

    ``None`` is retained for backward compatibility with the original helper;
    diagnostics are emitted to stderr. New code should generally use the
    returned ``code``/``stderr`` fields or the higher-level CLI.
    """
    cid = resolve_compiler(compiler_id)
    body = {
        "source": source,
        "lang": "c",
        "allowStoreCodeDebug": False,
        "options": {
            "userArguments": flags,
            "filters": {
                "binary": False,
                "binaryObject": False,
                "execute": False,
                "intel": intel,
                "demangle": True,
                "directives": True,
                "labels": True,
                "commentOnly": True,
                "trim": True,
                "libraryCode": False,
            },
            "compilerOptions": {"executorRequest": False, "skipAsm": False},
            "tools": [],
            "libraries": [],
        },
    }
    try:
        data = _request_json(f"{API}/compiler/{cid}/compile", body=body, timeout=timeout)
    except GodboltError as error:
        print(f"godbolt: {cid}: {error}", file=sys.stderr)
        return None
    if not isinstance(data, dict):
        print(f"godbolt: {cid}: malformed response", file=sys.stderr)
        return None
    if data.get("code") != 0:
        diagnostics = data.get("stderr") or data.get("stdout") or []
        rendered = "\n".join(
            item.get("text", str(item)) if isinstance(item, dict) else str(item)
            for item in diagnostics
        )
        print(f"godbolt: {cid}: compilation failed\n{rendered}", file=sys.stderr)
        return None
    return data


def assembly_lines(result: dict[str, Any]) -> list[str]:
    return [item.get("text", "") for item in (result.get("asm") or [])]


def _function_body(lines: list[str], wanted: str | None) -> list[str]:
    if not wanted:
        return lines
    # Handles foo:, "foo": (GCC CE output), and foo: # comments.
    label = re.compile(r'^\s*"?([^"\s:]+)"?:\s*(?:[#;].*)?$')
    out: list[str] = []
    active = False
    for line in lines:
        match = label.match(line)
        if match and not match.group(1).startswith("."):
            if active:
                break
            active = match.group(1) == wanted
        if active:
            out.append(line)
    return out


def _instruction_count(lines: list[str]) -> int:
    count = 0
    for line in lines:
        text = line.strip()
        if not text or text.startswith((".", "#", "//")) or text.endswith(":"):
            continue
        if re.match(r'^"?[^"\s:]+"?:', text):
            continue
        count += 1
    return count


def _compile_local(executable: str, source: Path, flags: str) -> list[str]:
    digest = hashlib.sha256((str(source.resolve()) + "\0" + flags).encode()).hexdigest()[:16]
    output = Path(os.environ.get("TMPDIR", "/tmp")) / f"lccc-godbolt-{digest}.s"
    command = [executable, *shlex.split(flags), "-S", str(source), "-o", str(output)]
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode:
        raise GodboltError(f"local compile failed: {' '.join(command)}\n{result.stderr}")
    try:
        return output.read_text().splitlines()
    finally:
        output.unlink(missing_ok=True)


def command_list(args: argparse.Namespace) -> int:
    pattern = re.compile(args.filter, re.IGNORECASE) if args.filter else None
    rows = []
    for item in list_compilers(args.language, refresh=args.refresh):
        text = f"{item.get('id', '')} {item.get('name', '')} {item.get('semver', '')}"
        if pattern and not pattern.search(text):
            continue
        rows.append(item)
    for item in rows:
        print(f"{item.get('id', ''):<24} {item.get('name', '')}")
    return 0


def command_compile(args: argparse.Namespace) -> int:
    result = compile_on_godbolt(args.compiler, args.source.read_text(), args.flags,
                                intel=args.intel)
    if result is None:
        return 1
    lines = _function_body(assembly_lines(result), args.function)
    print("\n".join(lines))
    return 0


def command_compare(args: argparse.Namespace) -> int:
    source_text = args.source.read_text()
    requested = [item.strip() for item in args.oracles.split(",") if item.strip()]
    compilers = list_compilers("c")
    artifact_dir: Path | None = args.artifact_dir
    if artifact_dir:
        artifact_dir.mkdir(parents=True, exist_ok=True)

    records: list[dict[str, Any]] = []
    try:
        local_lines = _compile_local(args.local, args.source, args.local_flags or args.flags)
        local_body = _function_body(local_lines, args.function)
        records.append({
            "key": "lccc", "id": str(Path(args.local)), "name": "local LCCC",
            "flags": args.local_flags or args.flags,
            "instructions": _instruction_count(local_body), "assembly": local_body,
        })
    except GodboltError as error:
        print(error, file=sys.stderr)
        return 1

    for requested_name in requested:
        cid = resolve_compiler(requested_name)
        meta = compiler_metadata(cid, compilers=compilers)
        result = compile_on_godbolt(cid, source_text, args.flags, intel=False)
        if result is None:
            records.append({"key": requested_name, **meta, "error": "compile failed"})
            continue
        body = _function_body(assembly_lines(result), args.function)
        records.append({
            "key": requested_name, **meta, "flags": args.flags,
            "instructions": _instruction_count(body), "assembly": body,
        })

    print(f"{'compiler':<14} {'resolved id':<20} {'instructions':>12}  name")
    print("-" * 82)
    for record in records:
        count = record.get("instructions")
        count_text = str(count) if count is not None else "ERROR"
        print(f"{record['key']:<14} {record['id']:<20} {count_text:>12}  {record.get('name', '')}")
        if artifact_dir and "assembly" in record:
            safe = re.sub(r"[^A-Za-z0-9_.-]+", "_", record["key"])
            (artifact_dir / f"{safe}.s").write_text("\n".join(record["assembly"]) + "\n")

    manifest = {
        "schema": 1,
        "source": str(args.source),
        "source_sha256": hashlib.sha256(source_text.encode()).hexdigest(),
        "function": args.function,
        "remote_flags": args.flags,
        "local_flags": args.local_flags or args.flags,
        "oracles": [{k: v for k, v in record.items() if k != "assembly"}
                    for record in records],
    }
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        tmp = args.json.with_suffix(f".tmp.{os.getpid()}")
        tmp.write_text(json.dumps(manifest, indent=2) + "\n")
        tmp.replace(args.json)
        print(f"wrote {args.json}")
    elif artifact_dir:
        (artifact_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return 1 if any("error" in record for record in records) else 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command")

    list_p = sub.add_parser("list", help="list CE C compilers")
    list_p.add_argument("--language", default="c")
    list_p.add_argument("--filter", default="")
    list_p.add_argument("--refresh", action="store_true")
    list_p.set_defaults(func=command_list)

    compile_p = sub.add_parser("compile", help="compile one source with one CE compiler")
    compile_p.add_argument("compiler")
    compile_p.add_argument("source", type=Path)
    compile_p.add_argument("--flags", default="-O3 -march=x86-64-v3")
    compile_p.add_argument("--function")
    compile_p.add_argument("--intel", action="store_true")
    compile_p.set_defaults(func=command_compile)

    compare_p = sub.add_parser("compare", help="compare local LCCC with all competition oracles")
    compare_p.add_argument("source", type=Path)
    compare_p.add_argument("--local", default="target/fastbuild/lccc")
    compare_p.add_argument("--flags", default="-O3 -march=x86-64-v3")
    compare_p.add_argument("--local-flags")
    compare_p.add_argument("--oracles", default=",".join(DEFAULT_ORACLES))
    compare_p.add_argument("--function")
    compare_p.add_argument("--artifact-dir", type=Path)
    compare_p.add_argument("--json", type=Path)
    compare_p.set_defaults(func=command_compare)
    return parser


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    # Backward-compatible spelling: godbolt.py <compiler-id> <source> ...
    if argv and argv[0] not in {"list", "compile", "compare", "-h", "--help"}:
        argv.insert(0, "compile")
    parser = build_parser()
    args = parser.parse_args(argv)
    if not hasattr(args, "func"):
        parser.print_help(sys.stderr)
        return 2
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
