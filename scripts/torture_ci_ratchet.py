#!/usr/bin/env python3
"""Fail closed on changed outcomes in an identity-matched torture campaign.

Baseline inventories must be copied from complete validated hosted reports,
with every non-pass identity (including skips) and no omitted rows. Any status
change, including a fix, requires explicit baseline review. Reference failures
are never allowed. Identity drift is an error, not permission to fall back to an
advisory success. This is NOT hermetic: host system headers, binutils, CRT,
shared libraries and kernel are not fully pinned by the comparison metadata.

Two kinds of identity. *What ran* -- corpus, case set, flags, target, runner
and harness digests, oracle, its search environment, the limits and the
operating system release the workflow asks for -- refuses the comparison when
it differs: the rows would be of another experiment. *Where it ran* -- the
kernel string and the build of the hosted image -- is the provider's choice
under one runner label, differs between two jobs of one run and changes every
week. It is recorded in the baseline, reported when it differs, and the rows
are compared all the same: a host that changes an outcome changes a row, and
a row is what this gate fails on. See HOST-IDENTITY-2026-11-26.md beside the
baselines for the thirteen runs that failed on nothing else.
"""
from __future__ import annotations

from collections import Counter
import json
import os
from pathlib import Path
import sys

from torture_ci_provenance import outcome_evidence, provenance
from torture_ci_summary import validated_report
from torture_evidence import STATUSES
from tools.corpus import schema


# Where a campaign ran, as opposed to what it ran: `host` is
# platform.platform() (the kernel build, `...-1014-aws` or `...-1017-azure`),
# `host_image` the runner image, whose `version` is the build of the day
# (`20261116.133.1` and `20261116.376.1` at once). `host_image.os` is the
# release `runs-on` names and stays part of what ran.
HOST_KEYS = ("host", "host_image")


def baseline_identity(baseline: object) -> dict:
    """The baseline's comparison identity, after the checks every use needs."""
    if not isinstance(baseline, dict) or type(baseline.get("schema")) is not int or baseline["schema"] != 1:
        raise ValueError("unsupported baseline schema")
    if set(baseline) != {"schema", "source_run", "source_head", "comparison", "counts", "outcomes"}:
        raise ValueError("missing or unknown baseline fields")
    if type(baseline["source_run"]) is not int or baseline["source_run"] <= 0:
        raise ValueError("baseline source run required")
    if not isinstance(baseline["source_head"], str) or not schema.SHA1.fullmatch(baseline["source_head"]):
        raise ValueError("baseline source head required")
    comparison = baseline["comparison"]
    if not isinstance(comparison, dict):
        raise ValueError("baseline comparison identity required")
    # A baseline says where it was captured, in the form a report does
    # (torture_ci_provenance.provenance): the host is not compared for
    # refusal, so nothing else would notice a baseline that had lost it.
    image = comparison.get("host_image")
    if (not isinstance(comparison.get("host"), str) or not comparison["host"] or
            not isinstance(image, dict) or set(image) != {"os", "version"} or
            not all(isinstance(image[key], str) and image[key] for key in image)):
        raise ValueError("baseline host and hosted image identity required")
    return comparison


def what_ran(comparison: dict) -> dict:
    """The identity that refuses a comparison: everything but where it ran."""
    strict = {key: value for key, value in comparison.items() if key not in HOST_KEYS}
    strict["host_image.os"] = comparison["host_image"]["os"]
    return strict


def host_drift(data: dict, baseline: object) -> list[str]:
    """Where the campaign ran, if not where the baseline was captured.

    For the reader of a failure, and of a success: the rows are compared in
    either case. Empty when the host is the baseline's.
    """
    before = baseline_identity(baseline)
    after = provenance(data)["comparison"]
    notes = []
    if before["host"] != after["host"]:
        notes.append(f"host: {before['host']} -> {after['host']}")
    if before["host_image"]["version"] != after["host_image"]["version"]:
        notes.append(f"host_image.version: {before['host_image']['version']} -> "
                     f"{after['host_image']['version']}")
    return notes


def compare(data: dict, baseline: object) -> list[str]:
    comparison = what_ran(baseline_identity(baseline))
    current = provenance(data)["comparison"]
    strict = what_ran(current)
    if comparison != strict:
        changed = sorted(key for key in set(comparison) | set(strict)
                         if comparison.get(key) != strict.get(key) or
                         (key in comparison) != (key in strict))
        return ["identity mismatch: " + ", ".join(changed)]
    rows = baseline["outcomes"]
    counts = baseline["counts"]
    if not isinstance(rows, list) or not isinstance(counts, dict):
        raise ValueError("baseline outcome rows and counts required")
    expected: dict[tuple[str, str], tuple[str, str, int, str]] = {}
    for row in rows:
        if not (isinstance(row, list) and len(row) == 6 and
                all(isinstance(value, str) and value for value in row[:4]) and
                type(row[4]) is int and isinstance(row[5], str) and schema.SHA256.fullmatch(row[5])):
            raise ValueError("invalid baseline outcome row")
        test, flags, status, phase, returncode, detail_sha256 = row
        if flags not in current["flags"] or status not in STATUSES - {"pass", "reference-fail"}:
            raise ValueError("invalid baseline flags or status (reference failures are never allowed)")
        if (test, flags) in expected:
            raise ValueError("duplicate baseline case")
        expected[test, flags] = (status, phase, returncode, detail_sha256)
    if any(not isinstance(key, str) or key not in STATUSES or type(value) is not int or value < 0
           for key, value in counts.items()):
        raise ValueError("invalid baseline counts")
    observed_counts = Counter(value[0] for value in expected.values())
    observed_counts["pass"] = current["expected_cases"] - len(expected)
    if observed_counts["pass"] <= 0 or any(counts.get(k, 0) != observed_counts.get(k, 0)
                                           for k in STATUSES):
        raise ValueError("baseline counts and outcome rows disagree")
    all_keys = {(row["test"], row["flags"]) for row in data["results"]}
    actual = {(row[0], row[1]): tuple(row[2:]) for row in outcome_evidence(data)}
    problems = []
    for key in sorted(set(expected) | set(actual)):
        before, after = expected.get(key), actual.get(key)
        if before != after:
            before_label = before[0] if before else "pass"
            after_label = after[0] if after else ("pass" if key in all_keys else "missing")
            suffix = " (phase, exit code, or diagnostic changed)" if before and after and before[0] == after[0] else ""
            problems.append(f"{json.dumps(key, ensure_ascii=True)}: {before_label} -> {after_label}{suffix}")
    return problems


def escaped(payload: dict) -> str:
    """One annotation line: JSON, with the workflow-command characters escaped."""
    text = json.dumps(payload, ensure_ascii=True, separators=(",", ":"))
    return text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def error_annotation(problems: list[str]) -> str:
    """Compact, escaped failure summary; full detail remains in job logs."""
    def encode(count: int) -> str:
        return escaped({"problems": problems[:count], "omitted": len(problems) - count})
    low, high = 0, len(problems)
    while low < high:
        middle = (low + high + 1) // 2
        if len(encode(middle).encode("utf-8")) <= 3800:
            low = middle
        else:
            high = middle - 1
    return encode(low)


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: torture_ci_ratchet.py REPORT BASELINE", file=sys.stderr)
        return 2
    try:
        data = validated_report(Path(argv[0]))
        baseline = schema.loads(Path(argv[1]).read_text(encoding="utf-8"))
        problems = compare(data, baseline)
        drift = host_drift(data, baseline)
    except (OSError, ValueError, schema.SchemaError) as exc:
        message = f"invalid torture ratchet input: {exc}"
        print(message, file=sys.stderr)
        if os.environ.get("GITHUB_ACTIONS"):
            print(f"::error title=GCC torture ratchet::{error_annotation([message])}")
        return 2
    if drift:
        # Said in both verdicts. With unchanged rows it is the record that
        # the baseline holds on this host too; with changed ones it is the
        # first thing a reviewer has to rule out.
        if os.environ.get("GITHUB_ACTIONS"):
            payload = escaped({"arch": data["arch"], "mode": data["mode"], "host_drift": drift})
            print(f"::notice title=GCC torture ratchet host::{payload}")
        print(f"torture ratchet: {data['arch']}/{data['mode']} ran on another host than the "
              "baseline's; rows compared all the same:")
        for note in drift:
            # Quoted: what the provider reports is not ours, and a line
            # break in it would start a workflow command of its own.
            print("  " + json.dumps(note, ensure_ascii=True))
    if problems:
        if os.environ.get("GITHUB_ACTIONS"):
            print(f"::error title=GCC torture ratchet::{error_annotation(problems)}")
        print("torture ratchet: comparison refused or outcomes changed:", file=sys.stderr)
        for problem in problems:
            print("  " + problem, file=sys.stderr)
        return 1
    print(f"torture ratchet: {data['arch']}/{data['mode']} matches reviewed per-case baseline")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
