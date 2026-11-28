#!/usr/bin/env python3
"""Inventory a public GitHub fork network without executing fork content.

  python3 scripts/audit_fork_network.py discover --out /tmp/heads.json
  python3 scripts/audit_fork_network.py inspect /tmp/heads.json \
      --cache /tmp/ccc-forks.git --fetch --out /tmp/deltas.json

Discovery recursively follows the public forks API and enumerates ALL advertised
branch heads, including topic branches on otherwise unchanged forks. Inspection
uses a separate bare Git repository, immutable commit IDs and exact tree deltas.
No checkout, submodule, hook, build, dependency install or foreign script runs.

Exit 2 with a written report means incomplete coverage (API/ref/fetch failure),
not a clean audit. Private/deleted forks, unadvertised refs, concurrent remote
changes and semantic correctness are outside this inventory's guarantees.
"""
from __future__ import annotations

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

REPOSITORY = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9_.-]+\Z")
OBJECT_ID = re.compile(r"[0-9a-f]{40}\Z")
ROOT = "anthropics/claudes-c-compiler"
TARGET = "ms178/lccc"
GIT_POLICY = ["-c", "core.hooksPath=/dev/null", "-c", "protocol.ext.allow=never"]


def run(args: list[str], *, timeout: int = 120, env: dict | None = None) -> bytes:
    result = subprocess.run(args, capture_output=True, timeout=timeout, env=env)
    if result.returncode:
        raise RuntimeError(result.stderr.decode("utf-8", "replace").strip()[:2000]
                           or f"command exited {result.returncode}")
    return result.stdout


def repository_name(name: str) -> str:
    if not isinstance(name, str) or not REPOSITORY.fullmatch(name) \
            or name.split("/")[1] in (".", ".."):
        raise ValueError(f"invalid public repository name: {name!r}")
    return name


def object_id(value: str) -> str:
    if not isinstance(value, str) or not OBJECT_ID.fullmatch(value):
        raise ValueError(f"not an immutable SHA-1 object ID: {value!r}")
    return value


def json_pages(text: str) -> list:
    """Older gh versions paginate as consecutive JSON values, without --slurp."""
    decoder = json.JSONDecoder()
    pages = []
    offset = 0
    while offset < len(text):
        if text[offset].isspace():
            offset += 1
            continue
        value, offset = decoder.raw_decode(text, offset)
        pages.append(value)
    if not pages:
        raise ValueError("empty GitHub API response")
    return pages


def github(endpoint: str, *, paginate: bool = False) -> list:
    # This is deliberately a PUBLIC census. The non-secret placeholder lets
    # old gh run without a login; the explicit anonymous header avoids sending
    # credentials or mistaking an authenticated private listing for coverage.
    args = ["gh", "api", "-H", "Authorization: anonymous"]
    if paginate:
        args.append("--paginate")
    args.append(endpoint)
    return json_pages(run(args, env={**os.environ, "GH_TOKEN": "public-read"})
                      .decode("utf-8"))


def parse_heads(raw: bytes) -> dict[str, str]:
    heads = {}
    for line in raw.decode("utf-8", "surrogateescape").splitlines():
        sha, ref = line.split("\t", 1)
        if not ref.startswith("refs/heads/") or ref == "refs/heads/":
            raise ValueError(f"unexpected advertised ref: {ref!r}")
        name = ref[len("refs/heads/"):]
        if name in heads:
            raise ValueError(f"duplicate branch: {name!r}")
        heads[name] = object_id(sha)
    return dict(sorted(heads.items()))


def list_heads(name: str) -> dict:
    url = f"https://github.com/{repository_name(name)}.git"
    try:
        raw = run(["git", *GIT_POLICY, "ls-remote", "--heads", url],
                  env={**os.environ, "GIT_TERMINAL_PROMPT": "0"})
        heads = parse_heads(raw)
        if not heads:
            raise ValueError("no advertised branch heads")
        return {"heads": heads}
    except (RuntimeError, ValueError, OSError, subprocess.TimeoutExpired) as error:
        return {"error": str(error)}


def discover(root: str, target: str, jobs: int) -> dict:
    root, target = repository_name(root), repository_name(target)
    repos = {}
    queue = [root]
    visited = set()
    errors = []
    while queue:
        name = queue.pop(0)
        if name.casefold() in visited:
            continue
        visited.add(name.casefold())
        try:
            if name not in repos:
                metadata, = github(f"repos/{name}")
                repos[name] = metadata
            pages = github(f"repos/{name}/forks?per_page=100&sort=newest", paginate=True)
            for page in pages:
                if not isinstance(page, list):
                    raise ValueError("forks API page is not a list")
                for record in page:
                    child = repository_name(record["full_name"])
                    repos[child] = record
                    if record.get("forks_count", 0):
                        queue.append(child)
        except (RuntimeError, ValueError, KeyError, OSError, subprocess.TimeoutExpired) as error:
            errors.append({"repository": name, "error": str(error)})
    names = sorted(repos)
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        refs = list(pool.map(list_heads, names))
    records = [{"full_name": name, "default_branch": repos[name]["default_branch"],
                **heads} for name, heads in zip(names, refs)]
    return {"schema": 1, "observed_utc": datetime.now(timezone.utc).isoformat(),
            "root": root, "target": target, "discovery_errors": errors,
            "complete": not errors and all("error" not in r for r in records),
            "repositories": records}


def git(cache: Path, *args: str) -> bytes:
    return run(["git", *GIT_POLICY, "-C", str(cache), *args])


def subsystem(path: str) -> str:
    if path.startswith("src/"):
        parts = path.split("/")
        return "/".join(parts[:3] if parts[1] in ("frontend", "backend", "ir") else parts[:2])
    return path.split("/", 1)[0]


def parse_delta(raw: bytes) -> list[dict]:
    """Parse --raw -z --no-renames, retaining modes and binary/path identity."""
    fields = raw.split(b"\0")
    if fields.pop() != b"" or len(fields) % 2:
        raise ValueError("malformed NUL-delimited Git delta")
    changes = []
    for index in range(0, len(fields), 2):
        old_mode, new_mode, old_sha, new_sha, status = fields[index].decode("ascii").split()
        if not old_mode.startswith(":") or status not in ("A", "D", "M", "T"):
            raise ValueError("unexpected raw Git delta record")
        changes.append({"path": fields[index + 1].decode("utf-8", "surrogateescape"),
                        "status": status, "old_mode": old_mode[1:], "new_mode": new_mode,
                        "old": object_id(old_sha), "new": object_id(new_sha)})
    return changes


def delta_summary(cache: Path, base: str, tip: str) -> dict:
    raw = git(cache, "diff", "--raw", "--abbrev=40", "-z", "--no-renames",
              "--no-ext-diff", "--no-textconv", base, tip, "--")
    changes = parse_delta(raw)
    source = [r["path"] for r in changes if r["path"].startswith(("src/", "include/"))]
    boundaries = [r["path"] for r in changes if r["path"] in
                  ("Cargo.toml", "Cargo.lock", "build.rs", "rust-toolchain.toml")
                  or r["path"].startswith((".cargo/", ".github/", "scripts/"))]
    unusual = [r["path"] for r in changes
               if r["new_mode"] not in ("000000", "100644", "100755")]
    source_entries = git(cache, "ls-tree", "-z", tip, "--", "src", "include")
    return {"tree": git(cache, "rev-parse", f"{tip}^{{tree}}").decode().strip(),
            "source_tree_sha256": hashlib.sha256(source_entries).hexdigest(),
            "changed_paths": len(changes), "delta_sha256": hashlib.sha256(raw).hexdigest(),
            "subsystems": dict(sorted(Counter(subsystem(r["path"]) for r in changes).items())),
            "source_paths": source, "execution_boundary_paths": boundaries,
            "nonregular_paths": unusual,
            "deleted_source_paths": [r["path"] for r in changes
                                     if r["status"] == "D" and r["path"] in source]}


def source_entries(cache: Path, revision: str) -> dict[str, bytes]:
    entries = {}
    for entry in git(cache, "ls-tree", "-r", "-z", revision, "--", "src", "include").split(b"\0"):
        if entry:
            metadata, path = entry.split(b"\t", 1)
            entries[path.decode("utf-8", "surrogateescape")] = metadata
    return entries


def inspect(network: dict, cache: Path, fetch: bool, against: str | None = None) -> dict:
    root = repository_name(network["root"])
    target = repository_name(network["target"])
    records = network["repositories"]
    if against is not None:
        object_id(against)
    if not cache.exists():
        if not fetch:
            raise ValueError("audit cache is absent (use --fetch to create it)")
        run(["git", *GIT_POLICY, "init", "--bare", "--", str(cache)])
    if git(cache, "rev-parse", "--is-bare-repository").strip() != b"true":
        raise ValueError("audit cache must be a separate bare Git repository")
    if against is not None and git(cache, "cat-file", "-t", against).strip() != b"commit":
        raise ValueError("--against must identify a commit, not another object type")
    roots = [r for r in records if r["full_name"].casefold() == root.casefold()]
    if len(roots) != 1:
        raise ValueError("inventory must contain exactly one source repository")
    base = object_id(roots[0].get("heads", {}).get(roots[0]["default_branch"]))
    refs = {}
    failures = list(network.get("discovery_errors", []))
    available = set()
    for record in records:
        name = repository_name(record["full_name"])
        if name.casefold() == target.casefold():
            continue  # Do not rediscover the target's historical research branches.
        if "error" in record:
            failures.append({"repository": name, "error": record["error"]})
        for branch, sha in record.get("heads", {}).items():
            object_id(sha)
            refs.setdefault(sha, []).append({"repository": name, "branch": branch})
            if sha in available:
                continue
            try:
                try:
                    git(cache, "cat-file", "-e", f"{sha}^{{commit}}")
                except RuntimeError:
                    if not fetch:
                        raise
                    # Immutable refs only, no submodules/tags/checkout; serialize
                    # fetches because shallow-file updates are repository-global.
                    git(cache, "-c", "fetch.fsckObjects=true", "fetch", "--no-tags",
                        "--no-write-fetch-head", "--no-recurse-submodules", "--depth=2",
                        f"https://github.com/{name}.git", f"+{sha}:refs/fork-audit/{sha}")
                available.add(sha)
            except (RuntimeError, OSError, subprocess.TimeoutExpired) as error:
                failures.append({"repository": name, "tip": sha, "error": str(error)})
    if base not in available:
        raise ValueError("source baseline commit is unavailable; no delta can be certified")
    current = source_entries(cache, object_id(against)) if against is not None else None
    tips = []
    for tip in sorted(available):
        summary = delta_summary(cache, base, tip)
        if current is not None:
            incoming = source_entries(cache, tip)
            counts = Counter()
            for path in summary["source_paths"]:
                new, here = incoming.get(path), current.get(path)
                if new == here:
                    status = "identical-entry" if new is not None else "absent-both"
                elif here is None:
                    status = "incoming-path-absent"
                elif new is None:
                    status = "incoming-deletion-local-present"
                else:
                    status = "diverged-review-required"
                counts[status] += 1
            summary["against_entry_counts"] = dict(sorted(counts.items()))
        tips.append({"tip": tip, "refs": refs[tip],
                     "subject": git(cache, "show", "-s", "--format=%s", tip).decode("utf-8", "replace").strip(),
                     **summary})
    return {"schema": 1, "observed_utc": network["observed_utc"], "root": root,
            "target_excluded": target, "baseline": base, "against": against,
            "complete": not failures,
            "warning": "Exact tree inventory is NOT semantic approval or an adoption proof.",
            "repositories": len(records), "distinct_tips": len(tips),
            "distinct_trees": len({t["tree"] for t in tips}),
            "distinct_source_trees": len({t["source_tree_sha256"] for t in tips}),
            "failures": failures, "tips": tips}


def write_report(path: Path, report: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent,
                                     prefix=f".{path.name}.", delete=False) as stream:
        temporary = Path(stream.name)
        try:
            json.dump(report, stream, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
            os.replace(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    census = commands.add_parser("discover")
    census.add_argument("--root", default=ROOT)
    census.add_argument("--target", default=TARGET)
    census.add_argument("--jobs", type=int, choices=range(1, 9), default=4)
    audit = commands.add_parser("inspect")
    audit.add_argument("inventory", type=Path)
    audit.add_argument("--cache", type=Path, required=True)
    audit.add_argument("--fetch", action="store_true")
    audit.add_argument("--against", help="immutable local commit already in the cache; exact source-entry comparison, not semantic adoption")
    for sub in (census, audit):
        sub.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    try:
        report = (discover(args.root, args.target, args.jobs) if args.command == "discover"
                  else inspect(json.loads(args.inventory.read_text()), args.cache, args.fetch, args.against))
        write_report(args.out, report)
        print(f"{'COMPLETE INVENTORY' if report['complete'] else 'INCOMPLETE COVERAGE'}: {args.out}")
        return 0 if report["complete"] else 2
    except (ValueError, KeyError, RuntimeError, OSError, subprocess.TimeoutExpired) as error:
        parser.exit(2, f"audit inventory error: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
