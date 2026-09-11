#!/usr/bin/env python3
"""Audit the historic byte-to-char UTF-8 peephole defect reproducibly.

MS-09 needs more than observing that today's shared helper is safe.  This
script proves the historical source shape and fix ancestry, scans all shipped
text-peephole rewrite surfaces for raw ``bytes[i] as char`` reconstruction,
checks the policy guard, and optionally records public GitHub distribution
provenance (releases, tags, Actions workflows, and artifacts).

The online mode deliberately describes only publicly observable official
publication.  It cannot make a claim about an arbitrary developer's private
build; the audit report states that scope explicitly.

Examples:
    scripts/audit_peephole_utf8.py --json /tmp/ms09-audit.json
    scripts/audit_peephole_utf8.py --online --json engineering/evidence/ms09.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
HISTORICAL_SHARED_FIX = "331f39c7da2f12d2115372ffd2bd558fa91859f4"
SHARED_CONSOLIDATION = "3494fd6ef03bed820a0258d6e5d5358bf1c35d33"
REPOSITORY_API = "https://api.github.com/repos/ms178/lccc"

# A byte-to-char cast used only for an ASCII predicate is not a UTF-8
# reconstruction.  This pattern flags the dangerous operation specifically:
# appending an indexed raw byte as a Rust char to a textual output buffer.
RAW_REBUILD_RE = re.compile(
    r"\b[A-Za-z_]\w*\.(?:push|push_str)\s*\(\s*"
    r"[A-Za-z_]\w*(?:\.as_bytes\(\))?\s*\[[^\]\n]+\]"
    r"\s+as\s+char\s*\)"
)
BYTE_CAST_RE = re.compile(r"\b[A-Za-z_]\w*(?:\.as_bytes\(\))?\s*\[[^\]\n]+\]\s+as\s+char")


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def atomic_write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp.{os.getpid()}")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def run_git(repo: Path, args: list[str], *, check: bool = True) -> str:
    completed = subprocess.run(
        ["git", *args],
        cwd=repo,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if check and completed.returncode != 0:
        raise RuntimeError(
            f"git {' '.join(args)} failed ({completed.returncode}): {completed.stderr.strip()}"
        )
    return completed.stdout


def historical_blob(repo: Path, revision: str, path: str) -> str:
    return run_git(repo, ["show", f"{revision}:{path}"])


def peephole_sources(repo: Path) -> list[Path]:
    """Return exactly the text-peephole source surfaces, not all byte parsers."""
    fixed = [
        repo / "src/backend/peephole_common.rs",
        repo / "src/backend/arm/codegen/peephole.rs",
        repo / "src/backend/riscv/codegen/peephole.rs",
        repo / "src/backend/i686/codegen/peephole.rs",
    ]
    x86_root = repo / "src/backend/x86/codegen/peephole"
    if x86_root.is_dir():
        fixed.extend(sorted(x86_root.rglob("*.rs")))
    return sorted(path for path in fixed if path.is_file())


def scan_current_sources(repo: Path) -> dict[str, Any]:
    raw_rebuilds: list[dict[str, Any]] = []
    all_byte_casts: list[dict[str, Any]] = []
    files: list[dict[str, str]] = []
    for path in peephole_sources(repo):
        rel = str(path.relative_to(repo))
        text = path.read_text(encoding="utf-8")
        files.append({"path": rel, "sha256": sha256_file(path)})
        for lineno, line in enumerate(text.splitlines(), start=1):
            if RAW_REBUILD_RE.search(line):
                raw_rebuilds.append({"path": rel, "line": lineno, "text": line.strip()})
            if BYTE_CAST_RE.search(line):
                all_byte_casts.append({"path": rel, "line": lineno, "text": line.strip()})
    return {
        "files": files,
        "raw_byte_to_char_rebuilds": raw_rebuilds,
        # Retain reviewable contexts for casts which are predicates, not output
        # reconstruction.  They need human review but do not weaken this gate.
        "remaining_byte_to_char_cast_contexts": all_byte_casts,
    }


def historic_upload_audit(repo: Path, revision: str) -> dict[str, Any]:
    paths = [
        line
        for line in run_git(repo, ["ls-tree", "-r", "--name-only", revision, "--", ".github/workflows"]).splitlines()
        if line.endswith((".yml", ".yaml"))
    ]
    uploads: list[dict[str, Any]] = []
    compiler_candidates: list[dict[str, Any]] = []
    for path in paths:
        text = historical_blob(repo, revision, path)
        lines = text.splitlines()
        for index, line in enumerate(lines):
            if "actions/upload-artifact" not in line:
                continue
            # A workflow step starts at the next list item.  Keep the complete
            # current step, including multiline `path: |` content.
            end = len(lines)
            for candidate in range(index + 1, len(lines)):
                if re.match(r"^\s*-\s+(?:name|uses|run):", lines[candidate]):
                    end = candidate
                    break
            block = lines[index:end]
            entry = {"path": path, "line": index + 1, "block": block}
            uploads.append(entry)
            if any(re.search(r"(?:^|[^a-z])lccc(?:[^a-z]|$)|target/(?:release|fastbuild)", x, re.I) for x in block):
                compiler_candidates.append(entry)
    return {
        "workflow_files": paths,
        "upload_steps": uploads,
        "compiler_artifact_upload_candidates": compiler_candidates,
    }


def api_get_all(url: str) -> list[dict[str, Any]]:
    items: list[dict[str, Any]] = []
    while url:
        request = urllib.request.Request(
            url,
            headers={
                "Accept": "application/vnd.github+json",
                "User-Agent": "lccc-ms09-peephole-utf8-audit",
                "X-GitHub-Api-Version": "2022-11-28",
            },
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            data = json.load(response)
            if not isinstance(data, list):
                raise RuntimeError(f"expected a list from {url}, got {type(data).__name__}")
            items.extend(data)
            link = response.headers.get("Link", "")
        next_url = None
        for part in link.split(","):
            if 'rel="next"' in part:
                match = re.search(r"<([^>]+)>", part)
                if match:
                    next_url = match.group(1)
        url = next_url or ""
    return items


def api_get_object(url: str) -> dict[str, Any]:
    request = urllib.request.Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "lccc-ms09-peephole-utf8-audit",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        data = json.load(response)
    if not isinstance(data, dict):
        raise RuntimeError(f"expected an object from {url}, got {type(data).__name__}")
    return data


def api_get_list_field(url: str, field: str) -> list[dict[str, Any]]:
    """Read GitHub's object-wrapped list endpoints (such as Actions artifacts)."""
    items: list[dict[str, Any]] = []
    while url:
        request = urllib.request.Request(
            url,
            headers={
                "Accept": "application/vnd.github+json",
                "User-Agent": "lccc-ms09-peephole-utf8-audit",
                "X-GitHub-Api-Version": "2022-11-28",
            },
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            data = json.load(response)
            if not isinstance(data, dict) or not isinstance(data.get(field), list):
                raise RuntimeError(f"expected list field {field!r} from {url}")
            items.extend(data[field])
            link = response.headers.get("Link", "")
        next_url = None
        for part in link.split(","):
            if 'rel="next"' in part:
                match = re.search(r"<([^>]+)>", part)
                if match:
                    next_url = match.group(1)
        url = next_url or ""
    return items


def online_distribution_audit() -> dict[str, Any]:
    """Capture concise public-distribution facts from the GitHub REST API."""
    try:
        metadata = api_get_object(REPOSITORY_API)
        releases = api_get_all(f"{REPOSITORY_API}/releases?per_page=100")
        tags = api_get_all(f"{REPOSITORY_API}/tags?per_page=100")
        workflows = api_get_object(f"{REPOSITORY_API}/actions/workflows?per_page=100")
        artifacts = api_get_list_field(f"{REPOSITORY_API}/actions/artifacts?per_page=100", "artifacts")
    except (OSError, urllib.error.URLError, urllib.error.HTTPError, json.JSONDecodeError, RuntimeError) as exc:
        return {"queried": True, "ok": False, "error": f"{type(exc).__name__}: {exc}"}

    compiler_artifacts = [
        {key: item.get(key) for key in ("id", "name", "size_in_bytes", "expired", "created_at", "updated_at")}
        for item in artifacts
        if "lccc" in str(item.get("name", "")).lower()
    ]
    return {
        "queried": True,
        "ok": True,
        "repository": {
            key: metadata.get(key)
            for key in (
                "full_name",
                "private",
                "created_at",
                "updated_at",
                "pushed_at",
                "default_branch",
                "has_downloads",
                "archived",
                "fork",
            )
        },
        "release_count": len(releases),
        "releases": [
            {key: release.get(key) for key in ("id", "tag_name", "name", "draft", "prerelease", "created_at")}
            for release in releases
        ],
        "tag_count": len(tags),
        "tags": [{key: tag.get(key) for key in ("name", "commit")} for tag in tags],
        "workflow_count": len(workflows.get("workflows", [])),
        "workflows": [
            {key: workflow.get(key) for key in ("id", "name", "path", "state")}
            for workflow in workflows.get("workflows", [])
        ],
        "artifact_count": len(artifacts),
        "compiler_named_artifacts": compiler_artifacts,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=ROOT, help="repository root (default: this script's parent)")
    parser.add_argument("--online", action="store_true", help="also query public GitHub distribution metadata")
    parser.add_argument("--json", type=Path, help="write result atomically as JSON")
    args = parser.parse_args()

    repo = args.repo.resolve()
    result: dict[str, Any] = {
        "audit": "MS-09 peephole UTF-8",
        "scope": (
            "shared and architecture-specific textual peephole rewrites; public official "
            "GitHub release/tag/Actions publication when --online is used"
        ),
        "out_of_scope": (
            "arbitrary private developer builds and non-peephole byte parsers/reconstructors; "
            "those require their own provenance or parser audit"
        ),
        "historical_fix": HISTORICAL_SHARED_FIX,
        "shared_consolidation": SHARED_CONSOLIDATION,
    }

    try:
        if not (repo / ".git").exists():
            raise RuntimeError(f"not a Git worktree: {repo}")
        result["head"] = run_git(repo, ["rev-parse", "HEAD"]).strip()
        result["origin_main"] = run_git(repo, ["rev-parse", "origin/main"]).strip()
        result["script_sha256"] = sha256_file(Path(__file__).resolve())
        result["sources"] = scan_current_sources(repo)

        old_shared = historical_blob(repo, f"{HISTORICAL_SHARED_FIX}^", "src/backend/peephole_common.rs")
        fixed_shared = historical_blob(repo, HISTORICAL_SHARED_FIX, "src/backend/peephole_common.rs")
        old_arm = historical_blob(repo, f"{SHARED_CONSOLIDATION}^", "src/backend/arm/codegen/peephole.rs")
        old_riscv = historical_blob(repo, f"{SHARED_CONSOLIDATION}^", "src/backend/riscv/codegen/peephole.rs")
        consumers = run_git(
            repo,
            ["grep", "-l", "peephole_common", f"{HISTORICAL_SHARED_FIX}^", "--", "src/backend"],
            check=False,
        ).splitlines()
        history = {
            "pre_fix_shared_has_raw_rebuild": bool(RAW_REBUILD_RE.search(old_shared)),
            "fix_commit_shared_has_raw_rebuild": bool(RAW_REBUILD_RE.search(fixed_shared)),
            "pre_consolidation_arm_has_raw_rebuild": bool(RAW_REBUILD_RE.search(old_arm)),
            "pre_consolidation_riscv_has_raw_rebuild": bool(RAW_REBUILD_RE.search(old_riscv)),
            "pre_fix_shared_consumers": consumers,
            "fix_is_ancestor_of_head": subprocess.run(
                ["git", "merge-base", "--is-ancestor", HISTORICAL_SHARED_FIX, "HEAD"], cwd=repo
            ).returncode
            == 0,
            "fix_is_ancestor_of_origin_main": subprocess.run(
                ["git", "merge-base", "--is-ancestor", HISTORICAL_SHARED_FIX, "origin/main"], cwd=repo
            ).returncode
            == 0,
        }
        result["history"] = history
        policy = (repo / "engineering/agent/RULES.md").read_text(encoding="utf-8")
        result["policy_guard_present"] = "peephole text-rewrite helper" in policy
        result["historic_workflow_audit"] = historic_upload_audit(repo, f"{HISTORICAL_SHARED_FIX}^")

        checks = {
            "current_peephole_raw_rebuilds_absent": not result["sources"]["raw_byte_to_char_rebuilds"],
            "history_reproducer_present": history["pre_fix_shared_has_raw_rebuild"],
            "historical_fix_removed_shared_rebuild": not history["fix_commit_shared_has_raw_rebuild"],
            "pre_consolidation_callers_were_vulnerable": (
                history["pre_consolidation_arm_has_raw_rebuild"]
                and history["pre_consolidation_riscv_has_raw_rebuild"]
            ),
            "historical_fix_reachable": (
                history["fix_is_ancestor_of_head"] and history["fix_is_ancestor_of_origin_main"]
            ),
            "policy_guard_present": result["policy_guard_present"],
            "historic_workflows_did_not_upload_compiler": not result["historic_workflow_audit"]["compiler_artifact_upload_candidates"],
        }
        if args.online:
            online = online_distribution_audit()
            result["online_distribution"] = online
            checks["online_query_succeeded"] = online.get("ok", False)
            if online.get("ok"):
                checks["no_public_releases"] = online["release_count"] == 0
                checks["no_public_tags"] = online["tag_count"] == 0
                checks["no_lccc_named_actions_artifacts"] = not online["compiler_named_artifacts"]
        result["checks"] = checks
        result["status"] = "PASS" if all(checks.values()) else "FAIL"
    except (OSError, RuntimeError, subprocess.SubprocessError) as exc:
        result["status"] = "FAIL"
        result["error"] = f"{type(exc).__name__}: {exc}"

    if args.json:
        atomic_write_json(args.json, result)
    print(
        f"{result['status']}: raw_rebuilds="
        f"{len(result.get('sources', {}).get('raw_byte_to_char_rebuilds', []))} "
        f"online={'yes' if args.online else 'no'}"
    )
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
