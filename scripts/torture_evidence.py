#!/usr/bin/env python3
"""Summarise gcc.c-torture JSON reports into the session evidence table.

Input:  one or more ``--json`` reports written by ``scripts/x86_gcc_torture.py``
        or ``scripts/x86_gcc_torture_i686.py``.  A schema-2 report is a
        *completion claim*, so it is validated as one: explicit schema, real
        integer cardinalities, declared configuration membership, unique
        ``(test, flags)`` keys, the exact case set the run owed, and counts
        that agree with the results they summarise.  Schema 1 is archival:
        it predates those fields, cannot be checked, and is only read when
        ``--allow-legacy-schema1`` is passed, with every row it produces
        labelled ``verified: false``.

Output: ``engineering/evidence/session/evidence.json`` (consumed by the
harness web console via ``scripts/lccc-harness-snapshot.sh``) and a
Markdown table on stdout for follow-up documents.

Reference skips (``reference-compile-skip`` / ``reference-run-skip``) are tests
the host GCC itself cannot build or run, and ``unsupported`` tests are the ones
gcc's own harness would not run either (``run_expensive_tests`` without
``GCC_TEST_RUN_EXPENSIVE``); all are excluded from the denominator because
they say nothing about LCCC.  Everything else is either ``pass`` or a
validation failure, so ``pass + fail == total``.  Reference tool/infrastructure
failures remain failures (never skips), but are separately counted as
``reference_fail`` and shown in their own column: an oracle ICE, an OOM-killed
reference, or a timeout is evidence that the *oracle* could not adjudicate the
case, not that LCCC miscompiled -- so it is never folded into ``lccc_fail``.

Refusing to summarise is the point.  A consumer that reconstructs
"1/1 pass, zero failures" from a truncated, duplicated, or future-format
report turns a hole in the evidence into a green row, which is the failure
this file was written to prevent.

Usage:
  scripts/torture_evidence.py x64.json:x86-64 i686.json:i686 [--out evidence.json]
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from tools.corpus import evidence, publication, schema  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
DEFAULT_OUT = REPO / "engineering" / "evidence" / "session" / "evidence.json"
CURRENT_SCHEMA = 2
LEGACY_SCHEMA = 1
MODES = ("execute", "compile")
# The producer's whole status vocabulary.  A status outside it means the report
# came from a newer or different producer, and silently bucketing an unknown
# status as "not a skip, so not counted anywhere" is how a failure disappears.
STATUSES = frozenset({
    "pass", "compile-fail", "link-fail", "run-fail", "reference-fail",
    "reference-compile-skip", "reference-run-skip", "unsupported",
})
SKIPS = frozenset({"reference-compile-skip", "reference-run-skip", "unsupported"})
# A reference failure is a validation failure of the *oracle*; it is not
# attributable to LCCC and must not be counted in `lccc_fail`.
REFERENCE_FAILURES = frozenset({"reference-fail"})
DEFECTS = (STATUSES - SKIPS - {"pass"}) - REFERENCE_FAILURES


def _is_count(value: object) -> bool:
    """A real non-negative integer.  ``True`` is ``1`` in Python; it is not a count."""
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def case_keys(results: list[dict]) -> list[str]:
    """The set of cases a report actually carries, in stable order."""
    return sorted(evidence.case_key(r["test"], r["flags"]) for r in results
                  if isinstance(r, dict) and isinstance(r.get("test"), str)
                  and isinstance(r.get("flags"), str))


def validate(data: object) -> list[str]:
    """Every way this report fails to be a trustworthy completion claim.

    Returns the list of problems instead of raising on the first one, so a
    producer bug shows up as a complete diagnosis rather than a sequence of
    one-line surprises.
    """
    errors: list[str] = []
    if not isinstance(data, dict):
        return ["report is not a JSON object"]
    # `report_schema`, never `schema`: this module imports `tools.corpus.schema`,
    # and a local of that name shadows it for the whole function body.
    report_schema = data.get("schema")
    if not _is_count(report_schema) or report_schema not in (CURRENT_SCHEMA, LEGACY_SCHEMA):
        # Deliberately not unlockable: `--allow-legacy-schema1` opens the
        # *older* format, whose missing checks we can describe, and must never
        # be the way a future or unknown schema gets summarised by guessing.
        return [f"unsupported or missing schema: {report_schema!r} (this build writes "
                f"{CURRENT_SCHEMA}; {LEGACY_SCHEMA} is archival and labelled; "
                f"anything else is refused, --allow-legacy-schema1 included)"]
    if report_schema != CURRENT_SCHEMA:
        return errors

    if data.get("complete") is not True:
        errors.append(f"incomplete torture checkpoint: complete is {data.get('complete')!r}")
    if data.get("fatal"):
        errors.append(f"fatal torture harness error recorded: {data['fatal']!r}")

    mode = data.get("mode")
    if mode not in MODES:
        errors.append(f"unknown mode {mode!r}; expected one of {list(MODES)}")

    # The row shape mirrors `Result` in scripts/x86_gcc_torture.py; a test in
    # tests/corpus/test_x86_torture_contracts.py asserts the two stay equal, so
    # a producer that grows a field trips the consumer loudly instead of
    # having that field ignored here.
    row_fields = (["test", "flags", "status"],
                  ["directive_flags", "phase", "returncode", "seconds", "detail"])
    flags = data.get("flags")
    try:
        schema.strings(flags)
    except schema.SchemaError as exc:
        errors.append(f"flags: {exc}")
        declared: list[str] = []
    else:
        declared = flags
        if not declared:
            # The empty-report class, first half: a report that declares no
            # configuration compiled nothing, and every count below is then a
            # count of nothing.  This used to validate clean -- the length rules
            # are satisfied by 0 == 0 == 0 and the digest was the digest of the
            # empty set, which a doctored or truncated file can produce exactly.
            errors.append("flags is empty: a report that declares no configuration has no "
                          "eligible cases to weigh, so it cannot be evidence of anything")
        if len(set(declared)) != len(declared):
            errors.append(f"duplicate configuration labels in flags: {declared}")

    results = data.get("results")
    if not isinstance(results, list):
        errors.append("results is not a list")
        return errors
    for key in ("expected_cases", "completed_cases"):
        try:
            schema.integer(data.get(key), minimum=0)
        except schema.SchemaError:
            errors.append(f"{key} must be a non-negative integer, got {data.get(key)!r}")
    if _is_count(data.get("expected_cases")) and _is_count(data.get("completed_cases")):
        if not (data["expected_cases"] == data.get("completed_cases") == len(results)):
            errors.append(f"case count mismatch: expected {data['expected_cases']}, "
                          f"completed {data['completed_cases']}, results {len(results)}")
    if data.get("complete") is True and _is_count(data.get("expected_cases")) \
            and data["expected_cases"] == 0:
        # A *checkpoint* may legitimately have run nothing yet, and it is refused
        # elsewhere for being incomplete; a report that claims completion while
        # owing zero cases is the shape of a selection that silently came up empty
        # (a moved suite directory, a filter that matched nothing).
        errors.append("expected_cases is 0 in a complete report: nothing was owed, so "
                      "nothing was measured, and a green row count here is vacuous")

    seen: set[str] = set()
    for index, row in enumerate(results):
        try:
            schema.keys(row, row_fields[0], row_fields[1])
        except schema.SchemaError as exc:
            errors.append(f"results[{index}]: {exc}")
            continue
        test, flag, status = row.get("test"), row.get("flags"), row.get("status")
        if not isinstance(test, str) or not test.strip():
            errors.append(f"results[{index}] has no usable test name: {test!r}")
            continue
        if status not in STATUSES:
            errors.append(f"results[{index}] ({test}) has unknown status {status!r}")
        if flag not in declared:
            # An undeclared configuration is how a failing -O3 result hides in a
            # report that declares only -O2: rows are built per declared flag,
            # so its failure would never reach a number.
            errors.append(f"results[{index}] ({test}) reports undeclared flags {flag!r}")
            continue
        key = f"{test}[{flag}]"
        if key in seen:
            errors.append(f"duplicate case in report: {key}")
        seen.add(key)
        for field in ("returncode", "seconds"):
            if field in row and isinstance(row[field], bool):
                errors.append(f"results[{index}].{field} is a bool, not a number")
        if "detail" in row and not isinstance(row["detail"], str):
            errors.append(f"results[{index}].detail must be text")

    expected = data.get("case_set_sha256")
    if not (isinstance(expected, str) and schema.SHA256.fullmatch(expected)):
        errors.append("case count mismatch: schema-2 report lacks case_set_sha256, so "
                      "a report of the right length could carry the wrong cases")
    elif evidence.case_set_digest(case_keys(results)) != expected:
        # Not gated on `seen`: with the gate, a report whose rows were stripped
        # skipped the very check that would have shown it was stripped, because no
        # case had been seen to compare.  The digest of the empty set is well
        # defined, so the comparison is meaningful in that corner too.
        errors.append("case set does not match the selection the run owed "
                      "(missing, extra or duplicated cases)")

    if data.get("dependency_digest_truncated"):
        # The producer already says so (`dependency_digest_truncated`, set when the
        # include walk hit DEPENDENCY_FILE_LIMIT); what was missing was a consumer
        # that refuses the claim.  A digest over *part* of the input set is not a
        # pin: editing a header past the cap -- the exact case the dependency digest
        # was added to catch -- leaves this report byte-for-byte valid.
        errors.append("dependency identity is truncated: headers beyond the cap are not "
                      "pinned, so this report cannot be published as verified evidence "
                      "(re-run with a larger cap or a narrower include set)")

    if data.get("complete") is True and declared:
        # Per-configuration coverage.  `expected_cases` is a total, so a report can
        # satisfy it (and the digest, if it was rebuilt for the reduced row set)
        # while one whole declared configuration contributes nothing: -O3 quietly
        # disappearing is not the same claim as -O3 passing everything.
        reported = {row.get("flags") for row in results if isinstance(row, dict)}
        missing = sorted(set(declared) - reported)
        if missing:
            errors.append(f"declared configuration(s) with no rows in a complete report: "
                          f"{missing} -- a configuration that owes cases and reports "
                          f"nothing was not measured, whatever the totals say")

    counts = data.get("counts")
    if counts is not None:
        if not isinstance(counts, dict) or any(
                not isinstance(k, str) or not _is_count(v) for k, v in counts.items()):
            errors.append("counts must map status names to non-negative integers")
        else:
            recomputed = Counter(r["status"] for r in results if isinstance(r, dict))
            if dict(recomputed) != {k: v for k, v in counts.items()}:
                errors.append(f"counts disagree with results: {dict(sorted(counts.items()))} "
                              f"vs {dict(sorted(recomputed.items()))}")
    return errors


def summarise(path: Path, target: str, *, allow_legacy: bool = True) -> tuple[list[dict], list[str]]:
    """Rows and notes for one report, or ``ValueError`` listing every defect.

    ``allow_legacy`` keeps archival (schema-1) reports readable, since the
    point of the archive is to be citable; callers that must not accept an
    unchecked report -- the command line, and therefore any CI job -- pass
    ``False``.  Legacy rows are marked ``verified: false`` and can never be
    mistaken for completion-checked evidence.
    """
    try:
        # `schema.loads`, not `json.loads`: a report with a duplicated
        # `"expected_cases"` key or a `NaN` count is corrupt, and last-value-wins
        # would have it read as a deliberate claim.
        data = schema.loads(path.read_text())
    except schema.SchemaError as exc:
        raise ValueError(f"{path}: unparseable report: {exc}") from exc
    errors = validate(data)
    report_schema = data.get("schema") if isinstance(data, dict) else None
    if errors and report_schema == CURRENT_SCHEMA:
        raise ValueError(f"{path}: " + "; ".join(errors))
    if report_schema not in (CURRENT_SCHEMA, LEGACY_SCHEMA):
        raise ValueError(f"{path}: " + "; ".join(errors))
    if report_schema != CURRENT_SCHEMA:
        if not allow_legacy:
            raise ValueError(
                f"{path}: archival schema-{report_schema} report; completion, "
                f"cardinality and case-set checks do not exist for that format.  "
                f"Pass --allow-legacy-schema1 to cite it as unverified evidence.")
        legacy_rows, legacy_notes = _rows(data, target, verified=False)
        return legacy_rows, [f"{target}: archival schema-{report_schema} report, "
                             f"completion not verified (no such check existed for that "
                             f"format); rows are labelled verified=false", *legacy_notes]
    return _rows(data, target, verified=True)


def _rows(data: dict, target: str, *, verified: bool) -> tuple[list[dict], list[str]]:
    per_flag: dict[str, Counter] = defaultdict(Counter)
    failing: dict[str, list[str]] = defaultdict(list)
    for r in data["results"]:
        per_flag[r["flags"]][r["status"]] += 1
        if r["status"] not in SKIPS and r["status"] != "pass":
            failing[r["test"]].append(f"{r['flags']}:{r['status']}")
    rows = []
    empty_flags = []
    for flag in data["flags"]:
        c = per_flag[flag]
        skipped = sum(v for k, v in c.items() if k in SKIPS)
        reference_fail = sum(v for k, v in c.items() if k in REFERENCE_FAILURES)
        lccc_fail = sum(v for k, v in c.items() if k in DEFECTS)
        fail = reference_fail + lccc_fail  # `pass + fail == total` stays true
        if c["pass"] + fail == 0:
            empty_flags.append(flag)
        rows.append(dict(
            suite="gcc.c-torture/" + data.get("mode", "execute"), target=target, flags=flag,
            pass_=c["pass"], fail=fail, lccc_fail=lccc_fail, total=c["pass"] + fail,
            skipped=skipped, reference_fail=reference_fail, verified=verified,
            note=f"lccc {str(data.get('lccc_head') or '')[:12]}; "
                 f"reference tool/infrastructure failures: {reference_fail}",
        ))
    selection = data.get("selection", {}) or {}
    partial = bool(data.get("from_list") or selection.get("tests") or selection.get("filter"))
    scope = "filtered selection" if partial else "recorded selection"
    if data.get("case_set_sha256") and data.get("schema") == CURRENT_SCHEMA:
        scope += ", case set verified against the selection"
    labels = "LCCC-side failure(s)"
    summary = (f"{target} ({scope}): {len(failing)} distinct failing sources: "
               + ", ".join(f"{t} [{' '.join(v)}]" for t, v in sorted(failing.items()))
               if failing else
               f"{target} ({scope}): zero reported failures across {', '.join(data['flags'])}")
    notes = [summary]
    if empty_flags:
        # An all-skipped configuration is not validation, whatever the counts
        # say: `0 failures / 0 cases` reads as success to everything downstream.
        notes.append(f"{target}: NO eligible cases for "
                     f"{', '.join(empty_flags)} -- a run of skips is not a pass "
                     f"rate; {labels} cannot be zero-rated on it")
    return rows, notes


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("reports", nargs="+", help="<report.json>:<target-label>")
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    ap.add_argument("--append-note", action="append", default=[])
    ap.add_argument("--allow-legacy-schema1", action="store_true",
                    help="cite archival schema-1 reports as unverified evidence")
    args = ap.parse_args(argv)
    rows, notes = [], []
    for item in args.reports:
        path, _, label = item.partition(":")
        try:
            r, n = summarise(Path(path), label or Path(path).stem,
                             allow_legacy=args.allow_legacy_schema1)
        except (OSError, ValueError, KeyError) as exc:
            ap.exit(2, f"cannot summarise {path}: {exc}\n")
        rows += r
        notes += n
    notes += args.append_note
    if not rows or not any(row["total"] for row in rows):
        # `if rows` used to guard this, so the emptiest possible input -- no rows
        # at all -- walked past the refusal, published `{"rows": []}`, and exited 0.
        # An empty evidence table and a table of skips are the same claim, and both
        # are the one thing this file exists to refuse.
        ap.exit(2, "no configuration with any eligible case was summarised; refusing to "
                   "publish an empty or skip-only evidence table\n")
    payload = {"generated": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
               "rows": [{("pass" if k == "pass_" else k): v for k, v in row.items()} for row in rows],
               "notes": notes}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    # The corpus publisher's helper: a per-destination temp name (two
    # consumers writing one .json.tmp used to be able to clobber each other),
    # fsync before visibility, and the previous file intact on failure.
    publication.atomic_bytes(args.out, (json.dumps(payload, indent=2, sort_keys=True,
                                                   allow_nan=False) + "\n").encode())
    print("| Target | Flags | Pass | Validation fail | Of which LCCC | Of which reference fail | Rate | Checked |")
    print("|---|---|---:|---:|---:|---:|---:|:-:|")
    for row in payload["rows"]:
        rate = 100.0 * row["pass"] / row["total"] if row["total"] else 0.0
        print(f"| {row['target']} | `{row['flags']}` | {row['pass']}/{row['total']} | "
              f"{row['fail']} | {row['lccc_fail']} | {row['reference_fail']} | "
              f"{rate:.2f}% | {'yes' if row['verified'] else 'legacy'} |")
    for n in notes:
        print(f"\n{n}")
    # Evidence of failures is evidence: it is published, but the exit status
    # says the campaign did not pass.
    return 1 if any(row["fail"] for row in payload["rows"]) else 0


if __name__ == "__main__":
    sys.exit(main())
