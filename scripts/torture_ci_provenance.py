#!/usr/bin/env python3
"""Publish the bounded identity needed to compare hosted torture inventories.

This is evidence for review, not a ratchet: it requires a validated complete
report and refuses missing or truncated identity fields. The recorded identity
still does not pin all host system headers, CRT, binutils, shared libraries,
kernel behaviour or full DejaGnu semantics. Candidate hashes are provenance,
not comparison keys: comparing compiler revisions is the point of a ratchet.
"""
from __future__ import annotations

import json
from pathlib import Path
import sys

from torture_ci_summary import validated_report
from tools.corpus import schema


def provenance(data: dict) -> dict:
    def digest(value, name):
        if not isinstance(value, str) or not schema.SHA256.fullmatch(value):
            raise ValueError(f"missing or invalid {name}")
        return value

    selection = data.get("selection")
    if not isinstance(selection, dict):
        raise ValueError("missing selection")
    if (selection.get("tests") != [] or selection.get("filter") != "" or
            selection.get("from_list") is not None):
        raise ValueError("provenance baseline requires an unfiltered whole corpus")
    if selection.get("dependency_digest_truncated") is not False:
        raise ValueError("dependency scan is missing or truncated")
    if type(selection.get("sources")) is not int or selection["sources"] <= 0:
        raise ValueError("invalid source count")
    if selection["sources"] * len(data["flags"]) != data["expected_cases"]:
        raise ValueError("source/configuration count does not match expected cases")
    if type(selection.get("dependency_files")) is not int or selection["dependency_files"] < 0:
        raise ValueError("invalid dependency count")
    digest(selection.get("selected_sources_sha256"), "selected source digest")
    digest(data.get("dependency_digest"), "dependency digest")
    digest(data.get("corpus_stamp_sha256"), "corpus stamp digest")
    digest(data.get("runner_sha256"), "runner digest")
    digest(data.get("oracle_environment_sha256"), "oracle environment digest")
    image = data.get("host_image")
    if not isinstance(image, dict) or not all(isinstance(image.get(k), str) and image[k] for k in ("os", "version")):
        raise ValueError("hosted image OS and version are required")
    host = data.get("host")
    if not isinstance(host, str) or not host:
        raise ValueError("host description required")
    tools = data.get("tools")
    if not isinstance(tools, dict) or not isinstance(tools.get("gcc"), dict) or not isinstance(tools.get("lccc"), dict):
        raise ValueError("oracle and candidate tool identities required")
    oracle = tools["gcc"]
    if oracle.get("version_returncode") != 0:
        raise ValueError("oracle version probe failed")
    digest(oracle.get("sha256"), "oracle driver digest")
    digest(tools["lccc"].get("sha256"), "candidate digest")
    if data.get("link_mode") not in ("lccc-ld", "driver"):
        raise ValueError("invalid link mode")
    for name in ("append", "runner"):
        if not isinstance(data.get(name), str):
            raise ValueError(f"missing {name}")
    if type(data.get("expensive")) is not bool:
        raise ValueError("missing expensive setting")
    for name in ("compile_timeout", "run_timeout", "jobs"):
        if type(data.get(name)) not in (int, float) or data[name] <= 0:
            raise ValueError(f"invalid {name}")
    env = data.get("environment")
    if not isinstance(env, dict) or any(not isinstance(k, str) or not isinstance(v, str) for k, v in env.items()):
        raise ValueError("invalid feature environment")
    comparison = {
        "arch": data["arch"], "mode": data["mode"], "corpus": data["gcc_checkout_head"],
        "flags": data["flags"], "case_set_sha256": data["case_set_sha256"],
        "expected_cases": data["expected_cases"], "selection": selection,
        "dependency_digest": data["dependency_digest"],
        "corpus_stamp_sha256": data["corpus_stamp_sha256"],
        "runner_sha256": data["runner_sha256"], "oracle_sha256": oracle["sha256"],
        "oracle_environment_sha256": data["oracle_environment_sha256"],
        "host_image": image, "host": host, "environment": env,
        "link_mode": data["link_mode"], "append": data["append"],
        "runner": data["runner"], "expensive": data["expensive"],
        "compile_timeout": data["compile_timeout"], "run_timeout": data["run_timeout"],
        "jobs": data["jobs"],
    }
    return {"comparison": comparison, "candidate_sha256": tools["lccc"]["sha256"]}


def main(argv: list[str]) -> int:
    if not argv:
        print("usage: torture_ci_provenance.py REPORT...", file=sys.stderr)
        return 2
    for name in argv:
        try:
            result = provenance(validated_report(Path(name)))
            payload = (json.dumps(result, separators=(",", ":"), ensure_ascii=True)
                       .replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A"))
            if len(payload.encode("utf-8")) > 3800:
                raise ValueError("provenance annotation exceeds safe limit")
        except (OSError, ValueError, schema.SchemaError) as exc:
            print(f"Invalid provenance {name}: {exc}", file=sys.stderr)
            return 2
        print(f"::notice title=GCC torture provenance::{payload}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
