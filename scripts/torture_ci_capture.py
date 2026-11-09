#!/usr/bin/env python3
"""Construct proposed baselines from GitHub API run/job/annotation JSON.

Inputs must be retrieved directly from GitHub using gh; JSON files themselves
are not cryptographically authenticated. Inspect and review the proposed diff,
including the full artifacts when available, before accepting a rebaseline.
"""
from __future__ import annotations

import argparse
from collections import Counter
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from tools.corpus import schema
from torture_evidence import STATUSES


ARCHES = ("x86_64", "i686")
MODES = ("compile", "execute")
REQUIRED_STEPS = ("Run full GCC 17 compile + execute corpus",
                  "Publish validated case inventory to check annotations",
                  "Publish whole-corpus provenance")
RATCHET = "Enforce whole-corpus per-case ratchet"


def capture(run: dict, jobs: dict, annotations: dict[int, list], *, allow_ratchet_failure: bool = False) -> dict[str, dict]:
    if (not isinstance(run, dict) or type(run.get("id")) is not int or run["id"] <= 0 or
            not isinstance(run.get("head_sha"), str) or not schema.SHA1.fullmatch(run["head_sha"]) or
            run.get("status") != "completed" or run.get("conclusion") not in ("success", "failure") or
            run.get("path") != ".github/workflows/gcc17-torture.yml" or
            not isinstance(run.get("repository"), dict) or run["repository"].get("full_name") != "ms178/lccc"):
        raise ValueError("invalid or incomplete GCC torture GitHub run metadata")
    if not isinstance(jobs, dict) or not isinstance(jobs.get("jobs"), list):
        raise ValueError("missing GitHub jobs")
    selected = {}
    for job in jobs["jobs"]:
        if not isinstance(job, dict):
            raise ValueError("malformed job")
        for arch in ARCHES:
            name = f"GCC 17 torture — {arch} / GCC 17 torture — {arch}"
            if job.get("name") != name:
                continue
            if arch in selected:
                raise ValueError("duplicate architecture job")
            if (type(job.get("id")) is not int or job.get("run_id") != run["id"] or
                    job.get("head_sha") != run["head_sha"] or job.get("status") != "completed"):
                raise ValueError("job does not belong to the completed run/head")
            steps = job.get("steps")
            if not isinstance(steps, list):
                raise ValueError("missing job steps")
            step_map = {}
            for step in steps:
                if not isinstance(step, dict) or not isinstance(step.get("name"), str) or step["name"] in step_map:
                    raise ValueError("invalid or duplicate job step")
                step_map[step["name"]] = step.get("conclusion")
            if any(step_map.get(name) != "success" for name in REQUIRED_STEPS):
                raise ValueError("campaign or publication did not succeed")
            ratchet = step_map.get(RATCHET)
            if ratchet == "failure":
                if not allow_ratchet_failure:
                    raise ValueError("ratchet failed; explicit review opt-in required")
            elif ratchet not in (None, "success"):
                raise ValueError("ratchet did not run successfully")
            if job.get("conclusion") not in ("success", "failure") or any(
                    value not in ("success", "skipped") for key, value in step_map.items() if key != RATCHET):
                raise ValueError("other job step failed or did not complete")
            if (job.get("conclusion") == "failure") != (ratchet == "failure"):
                raise ValueError("job conclusion does not match ratchet outcome")
            if ratchet == "failure" and run.get("conclusion") != "failure":
                raise ValueError("run conclusion does not match failed job")
            selected[arch] = (job["id"], ratchet)
    if set(selected) != set(ARCHES):
        raise ValueError("both architecture jobs are required")

    output = {}
    for arch, (job_id, ratchet) in selected.items():
        records = annotations.get(job_id)
        if not isinstance(records, list):
            raise ValueError(f"missing annotations for job {job_id}")
        collected = {mode: {"parts": {}} for mode in MODES}
        ratchet_errors = []
        for annotation in records:
            if not isinstance(annotation, dict):
                raise ValueError("malformed annotation")
            title = annotation.get("title")
            if title not in ("GCC torture ratchet", "GCC torture inventory", "GCC torture provenance", "GCC torture outcome evidence"):
                continue
            message = annotation.get("message")
            if not isinstance(message, str):
                raise ValueError("annotation message must be text")
            value = schema.loads(message)
            if title == "GCC torture ratchet":
                ratchet_errors.append(value)
                continue
            if not isinstance(value, dict):
                raise ValueError("annotation is not an object")
            identity = value.get("comparison") if title.endswith("provenance") else value
            if not isinstance(identity, dict) or identity.get("arch") != arch or identity.get("mode") not in MODES:
                raise ValueError("annotation target mismatch")
            entry = collected[identity["mode"]]
            if title.endswith("evidence"):
                part, parts = value.get("part"), value.get("parts")
                if type(part) is not int or type(parts) is not int or not 1 <= part <= parts <= 40 or part in entry["parts"]:
                    raise ValueError("invalid or duplicate outcome chunk")
                entry["parts"][part] = value
            else:
                key = title.split()[-1]
                if key in entry:
                    raise ValueError("duplicate inventory/provenance")
                entry[key] = value
        if ratchet == "failure" and (not ratchet_errors or any(
                not isinstance(e, dict) or type(e.get("omitted")) is not int or e["omitted"] != 0 or
                not isinstance(e.get("problems"), list) or not e["problems"] or
                any(not isinstance(p, str) or not p for p in e["problems"]) for e in ratchet_errors)):
            raise ValueError("ratchet failure lacks complete explanatory annotations")
        for mode, entry in collected.items():
            if set(entry) != {"parts", "inventory", "provenance"}:
                raise ValueError("missing inventory or provenance")
            inventory = entry["inventory"]
            comparison = entry["provenance"].get("comparison")
            if (not isinstance(comparison, dict) or inventory.get("nonpass_omitted") != 0 or
                    any(inventory.get(k) != comparison.get(k) for k in ("arch", "mode", "corpus", "flags", "case_set_sha256"))):
                raise ValueError("inventory/provenance mismatch or omission")
            parts = entry["parts"]
            if 1 not in parts:
                raise ValueError("outcome chunks missing")
            total = parts[1]["parts"]
            if set(parts) != set(range(1, total + 1)) or any(part["parts"] != total for part in parts.values()):
                raise ValueError("outcome chunks incomplete")
            outcomes = []
            for index in range(1, total + 1):
                rows = parts[index].get("rows")
                if not isinstance(rows, list):
                    raise ValueError("invalid outcome chunk")
                outcomes.extend(rows)
            if any(not isinstance(r, list) or len(r) != 6 or
                   not all(isinstance(v, str) and v for v in r[:4]) or type(r[4]) is not int or
                   not isinstance(r[5], str) or not schema.SHA256.fullmatch(r[5]) or
                   r[2] not in STATUSES - {"pass", "reference-fail"} for r in outcomes):
                raise ValueError("invalid outcome row or reference failure")
            keys = [(r[0], r[1]) for r in outcomes]
            if len(set(keys)) != len(keys) or sorted(r[:3] for r in outcomes) != inventory.get("nonpass"):
                raise ValueError("outcome and inventory identities disagree")
            counts = inventory.get("counts")
            expected_cases = comparison.get("expected_cases")
            if (not isinstance(counts, dict) or type(expected_cases) is not int or expected_cases <= len(outcomes) or
                    any(k not in STATUSES or type(v) is not int or v < 0 for k, v in counts.items())):
                raise ValueError("invalid case counts")
            actual_counts = Counter(r[2] for r in outcomes)
            actual_counts["pass"] = expected_cases - len(outcomes)
            if any(counts.get(k, 0) != actual_counts.get(k, 0) for k in STATUSES):
                raise ValueError("outcome and inventory counts disagree")
            output[f"{arch}-{mode}.json"] = {
                "schema": 1, "source_run": run["id"], "source_head": run["head_sha"],
                "comparison": comparison, "counts": counts, "outcomes": sorted(outcomes),
            }
    return output


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, required=True, help="GitHub run API JSON")
    parser.add_argument("--jobs", type=Path, required=True, help="GitHub jobs API JSON")
    parser.add_argument("--annotations-dir", type=Path, required=True, help="JOB_ID.json files from check-run annotations API")
    parser.add_argument("--out-dir", type=Path, required=True, help="new empty directory for proposed baselines")
    parser.add_argument("--allow-ratchet-failure", action="store_true", help="require complete ratchet error annotations and review them")
    args = parser.parse_args(argv)
    try:
        run = schema.loads(args.run.read_text())
        jobs = schema.loads(args.jobs.read_text())
        annotations = {}
        if isinstance(jobs, dict) and isinstance(jobs.get("jobs"), list):
            for job in jobs["jobs"]:
                if (isinstance(job, dict) and type(job.get("id")) is int and
                        isinstance(job.get("name"), str) and "GCC 17 torture —" in job["name"]):
                    annotations[job["id"]] = schema.loads((args.annotations_dir / f"{job['id']}.json").read_text())
        baselines = capture(run, jobs, annotations, allow_ratchet_failure=args.allow_ratchet_failure)
        args.out_dir.mkdir(parents=True, exist_ok=True)
        if any(args.out_dir.iterdir()):
            raise ValueError("output directory must be empty")
        for name, data in sorted(baselines.items()):
            (args.out_dir / name).write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
            print(args.out_dir / name)
        return 0
    except (OSError, ValueError, schema.SchemaError) as exc:
        print(f"cannot capture torture baseline: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
