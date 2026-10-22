#!/usr/bin/env python3
"""Divide a CI lane into balanced shards, from measured cost.

A lane's wall clock is the sum of its gates, so the only lever that shortens it
without dropping coverage is running independent gates concurrently.  Doing
that by hand is where the previous attempt went wrong: the split looked even
(2 x 191 s predicted) and ran 234 s / 527 s, because one gate's recorded cost
was a measurement from a broken workspace.  This tool therefore takes the cost
table as an input, refuses to guess, and records the assignment it chose so the
result can be audited later.

Three shapes, because a lane needs to be re-split as often as it is split:

``--mode steps``   The lane is a list of YAML steps (each gate is its own step,
                   like `frontend` and `codegen`).  The job is replaced by N
                   jobs whose preamble -- every step up to and including
                   `Verify exact compiler build` -- is copied byte for byte, so
                   a shard cannot quietly lose a toolchain cache and pay apt.
                   A ``Surface ...`` annotation step is glued to the step whose
                   log it reads, because splitting them apart would run the
                   annotator in a shard where the log does not exist.

``--mode shell``   The lane is one `run:` body of shell commands (the contract
                   shards).  Every command travels as a whole logical command
                   with the comment block that introduces it; the wrapper
                   (`set -Eeuo pipefail`, the ERR trap, the tee and annotation
                   tail) is rewritten identically into each shard.

``--mode join``    The inverse of ``--mode steps``: `<job>-a`, `<job>-b`, ...
                   become `<job>` again, including the job's AFTER-steps tail
                   (this workflow spells `needs:` there) and a `needs:` rewrite
                   that is a set, not a multiset.  A lane is re-balanced by
                   joining it and splitting it again, never by hand.

Assignment is longest-processing-time first over the measured costs, which is
the only ordering-independent rule: a gate's position in the file is an
artifact of when it was added, not of what it costs.

Every run rewrites `.github/ci-lane-plan.json`, the record
`tests/regression/check_ci_lane_plan.py` audits against the workflow.

    scripts/ci_lane_split.py --mode steps --job codegen --costs costs.tsv --write
    scripts/ci_lane_split.py --mode shell --lane-prefix "Verify remaining fast \\
        local contracts" --costs gates.tsv --write
    scripts/ci_lane_split.py --mode join --job codegen --write
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/ci.yml"
PLAN = ROOT / ".github/ci-lane-plan.json"
PREAMBLE_END = "Verify exact compiler build"
GLUE_PREFIX = "Surface "
LETTERS = "abcdefgh"
# A gate that nobody measured still costs a process spawn and a log write;
# pricing it at zero is what made the broken split look balanced.
MIN_STEP_SECONDS = 0.25
# The budget note this tool writes above a shard's gates.  Every line carries
# this tag, so the note can be removed EXACTLY -- without eating the
# hand-written comment that introduces the next gate -- and regenerated on
# every run: left in place, a merge-back and re-split would stack two notes
# describing two different splits, and the file would contain a comment that
# no run performed.
NOTE_TAG = "# lane-split: "
NOTE_INDENT = "    " + NOTE_TAG


# ── cost table ───────────────────────────────────────────────────────────────
def read_costs(path: Path) -> dict[str, float]:
    """`seconds<TAB>command` lines -> cost by normalised command text."""
    table: dict[str, float] = {}
    for line in path.read_text().splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) < 2:
            continue
        key = " ".join(parts[1].split())
        try:
            table[key] = max(table.get(key, 0.0), float(parts[0]))
        except ValueError:
            continue
    return table


def step_cost(block: list[str], table: dict[str, float]) -> float:
    """Measured seconds for one step block, from either cost-table keying.

    `--mode shell` measures logical commands and keys the table by command
    text; `--mode steps` needs a cost per step.  Reading only the step NAME
    made every step of a command-keyed table weigh nothing -- the first
    version priced a 9 s gate at 0.0 and produced a "balanced" split of a lane
    it had never measured.  So: the step name wins when it is present (a
    hand-written table may be per step), otherwise the commands the step runs
    are summed.  A key that a longer matched key contains is not counted
    twice, because an invocation that embeds another measured command would
    otherwise pay for both.
    """
    name = step_name(block)
    if name in table:
        return table[name]
    haystack = " ".join(" ".join(block).split())
    matched = [key for key in table if key and key in haystack]
    keep = [key for key in matched if not any(key != other and key in other
                                              for other in matched)]
    return sum(table[key] for key in keep)


def lpt(weights: list[float], shards: int) -> list[int]:
    totals = [0.0] * shards
    assignment = [0] * len(weights)
    for index in sorted(range(len(weights)), key=lambda i: (-weights[i], i)):
        target = min(range(shards), key=lambda s: (totals[s], s))
        assignment[index] = target
        totals[target] += weights[index]
    return assignment


# ── shared YAML-line helpers ─────────────────────────────────────────────────
def job_spans(lines: list[str]) -> dict[str, tuple[int, int]]:
    start = next(i for i, line in enumerate(lines) if line.rstrip() == "jobs:")
    spans: dict[str, tuple[int, int]] = {}
    current: str | None = None
    begin = start + 1
    for index in range(start + 1, len(lines)):
        line = lines[index]
        if not line.strip() or line.startswith("  #"):
            continue
        if re.match(r"^  [A-Za-z0-9_-]+:(\s|$)", line):
            if current is not None:
                spans[current] = (begin, index)
            current = line.split(":")[0].strip()
            begin = index
        elif re.match(r"^[A-Za-z0-9_'\"]", line):
            if current is not None:
                spans[current] = (begin, index)
            return spans
    if current is not None:
        spans[current] = (begin, len(lines))
    return spans


def replace_needs(lines: list[str], old: str, new_ids: list[str]) -> list[str]:
    """Point every `needs:` at the shards instead of the lane it replaced."""
    out: list[str] = []
    index = 0
    while index < len(lines):
        line = lines[index]
        match = re.match(r"^(\s*)needs:[ \t]*(.*)$", line)
        if not match:
            out.append(line)
            index += 1
            continue
        indent, value = match.group(1), match.group(2).strip()
        if value.startswith("[") and value.endswith("]"):
            items = [item.strip() for item in value[1:-1].split(",") if item.strip()]
            expanded = _dedupe(n for item in items for n in (new_ids if item == old else [item]))
            out.append(f"{indent}needs: [{', '.join(expanded)}]\n")
            index += 1
            continue
        # Block form: buffer the items so the rewrite stays a set, not a
        # multiset.  Expanding `frontend` -> `frontend-a, frontend-b` in a lane
        # that is joined and re-split twice used to leave four copies of the
        # same shard in `test.needs`: valid YAML, meaningless DAG, and
        # `test_parallel_ci_layout` only caught it because it counts them.
        out.append(line)
        index += 1
        items: list[str] = []
        while index < len(lines) and re.match(r"^\s*- [A-Za-z0-9_-]+$", lines[index].rstrip()):
            item = lines[index].strip()[2:]
            items.extend(new_ids if item == old else [item])
            index += 1
        out.extend(f"{indent}- {n}\n" for n in _dedupe(items))
    return out


def _dedupe(items) -> list[str]:
    """Order-preserving unique: a `needs:` list is a set, never a census."""
    seen: set[str] = set()
    out: list[str] = []
    for item in items:
        if item not in seen:
            seen.add(item)
            out.append(item)
    return out


def _display(path: Path) -> str:
    """Path as the reviewer knows it: repo-relative when inside the repo.

    A run against a copy (`--workflow /tmp/x.yml`) is exactly how the splitter
    is tested, so it must not raise on `relative_to` -- an exception there
    would make the documented `--workflow` flag unusable.
    """
    try:
        return str(path.resolve().relative_to(ROOT))
    except ValueError:
        return str(path)


def write_plan(plan_path: Path, workflow_path: Path, mode: str, group: str,
               shards: dict[str, dict], provenance: dict) -> None:
    record = json.loads(plan_path.read_text()) if plan_path.is_file() else {}
    # `mode` is a property of a LANE, not of the file: the same plan holds
    # `frontend-a` (gates are steps) beside `backend-contracts-a` (gates are
    # commands inside one run body).  A single top-level mode made the audit
    # read the contract shards with the step grammar -- both were "unplanned
    # gates" -- and `setdefault` kept whichever group was split first, so the
    # record was right only by accident of ordering.
    record.pop("mode", None)
    record.setdefault("provenance", {})[group] = provenance
    for lane, spec in shards.items():
        spec["mode"] = mode
    record.setdefault("lanes", {}).update(shards)
    # Drop lanes that no longer exist in the workflow, so the record cannot
    # accumulate stale shards that nothing runs.  The workflow to test is the
    # one this run rewrote (`--workflow`), not the module default: the audit
    # reads the same file, and a run against a copy must not prune lanes the
    # real workflow still has.
    workflow_text = workflow_path.read_text()
    for lane in list(record["lanes"]):
        if not re.search(rf"^  {re.escape(lane)}:", workflow_text, re.MULTILINE):
            del record["lanes"][lane]
    plan_path.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")


# ── mode: steps ─────────────────────────────────────────────────────────────
def _without_generated_note(lines: list[str]) -> list[str]:
    """Drop a previous run's tagged budget note, line by line.

    A tagged line is unmistakably ours, so the strip cannot swallow the
    hand-written comment that introduces the next gate (the untagged version
    did exactly that, and the comment was then absent from every shard).
    """
    return [line for line in lines if not line.startswith(NOTE_INDENT)]


def note(*parts: str) -> list[str]:
    """A budget note as YAML comments, one tagged line per `parts` entry."""
    return [NOTE_INDENT + part + "\n" for part in parts]


def step_blocks(lines: list[str], span: tuple[int, int]):
    """(header, step blocks, job tail) for one job."""
    begin, end = span
    header: list[str] = []
    index = begin
    while index < end and lines[index].rstrip() != "    steps:":
        header.append(lines[index])
        index += 1
    if index == end:
        raise SystemExit(f"job at line {begin + 1} has no `steps:`")
    header.append(lines[index])
    index += 1

    stop = end
    for probe in range(index, end):
        if re.match(r"^    [A-Za-z_][A-Za-z0-9_-]*:", lines[probe]):
            stop = probe
            break
    tail = lines[stop:end]

    blocks: list[list[str]] = []
    pending: list[str] = []
    for line in _without_generated_note(lines[index:stop]):
        if re.match(r"^    - ", line):
            blocks.append(pending + [line])
            pending = []
        elif not blocks:
            header.append(line)
        elif re.match(r"^    #", line):
            pending.append(line)
        elif pending and not line.strip():
            pending.append(line)
        elif pending:
            raise SystemExit(f"unexpected line after a comment block: {line!r}")
        else:
            blocks[-1].append(line)
    if pending:
        raise SystemExit("trailing comment block after the last step")
    return header, blocks, tail


def step_name(block: list[str]) -> str:
    for line in block:
        match = re.match(r"^    - name: (.*)$", line)
        if match:
            return match.group(1).strip()
    return "<unnamed>"


def split_steps(args) -> int:
    text = args.workflow.read_text()
    lines = text.splitlines(keepends=True)
    spans = job_spans(lines)
    if args.job not in spans:
        raise SystemExit(f"no job {args.job!r}")
    for shard in range(args.shards):
        if f"{args.job}-{LETTERS[shard]}" in spans:
            raise SystemExit(f"job {args.job}-{LETTERS[shard]} already exists")
    if not any(line.rstrip() == "    steps:" for line in lines[spans[args.job][0]:spans[args.job][1]]):
        raise SystemExit(f"job {args.job!r} has no steps")

    header, blocks, tail = step_blocks(lines, spans[args.job])
    label = next((m.group(1).strip() for m in
                  (re.match(r"^    name: (.*)$", line) for line in header) if m), args.job)
    # A lane that was merged back for a re-split still carries its old suffix;
    # appending another one produces "Contracts (shard b of 2) (shard a of 2)".
    label = re.sub(r"\s*\(shard [a-z] of \d+\)$", "", label)
    names = [step_name(block) for block in blocks]
    if PREAMBLE_END not in names:
        raise SystemExit(f"preamble marker {PREAMBLE_END!r} missing from {args.job}")
    cut = names.index(PREAMBLE_END) + 1
    preamble, gates = blocks[:cut], blocks[cut:]

    units: list[list[int]] = []
    for offset, block in enumerate(gates):
        if units and step_name(block).startswith(GLUE_PREFIX):
            units[-1].append(offset)
        else:
            units.append([offset])

    table = read_costs(args.costs)
    weights, missing = [], []
    for unit in units:
        # One floor, applied per STEP, so the record and the total agree:
        # `predicted_gate_seconds` is the sum of the `measured_s` entries a
        # reviewer sees, and a step nobody measured is priced at the same
        # floor the plan prints for it.
        costs = [max(step_cost(gates[i], table), MIN_STEP_SECONDS) for i in unit]
        if not any(step_cost(gates[i], table) for i in unit):
            missing.append(step_name(gates[unit[0]]))
        weights.append(sum(costs))
    if missing:
        print(f"warning: {len(missing)} step(s) unmeasured (weighed 0.25 s each): "
              + "; ".join(missing[:5]), file=sys.stderr)

    assignment = lpt(weights, args.shards)
    per_shard = [sum(w for w, a in zip(weights, assignment) if a == s) for s in range(args.shards)]

    out = lines[:spans[args.job][0]]
    shard_record: dict[str, dict] = {}
    for shard in range(args.shards):
        letter = LETTERS[shard]
        job_id = f"{args.job}-{letter}"
        shard_header = [line.replace(f"  {args.job}:", f"  {job_id}:", 1) for line in header]
        shard_header = [re.sub(r"^    name: .*$",
                               f"    name: {label} (shard {letter} of {args.shards})", line, count=1)
                        if line.startswith("    name: ") else line for line in shard_header]
        body, chosen = [], []
        for unit, where in zip(units, assignment):
            if where != shard:
                continue
            for offset in unit:
                body.extend(gates[offset])
                chosen.append({"name": step_name(gates[offset]),
                               "measured_s": round(max(step_cost(gates[offset], table),
                                                       MIN_STEP_SECONDS), 2)})
        out.extend(shard_header)
        for block in preamble:
            out.extend(block)
        out.extend(note(
            f"{len(chosen)} of {len(gates)} gate steps of the `{args.job}` lane, assigned",
            f"by longest-processing-time over measured cost ({per_shard[shard]:.0f} s of",
            f"{sum(per_shard):.0f} s). The preamble above is byte-identical in every",
            "shard; tests/regression/check_ci_lane_plan.py fails if a gate disappears,",
            "is duplicated, or a shard drifts from this record."))
        out.extend(body)
        out.extend(tail)
        shard_record[job_id] = {"group": args.job, "shards": args.shards,
                                "predicted_gate_seconds": round(per_shard[shard], 2),
                                "steps": chosen}
    out.extend(lines[spans[args.job][1]:])
    out = replace_needs(out, args.job, [f"{args.job}-{LETTERS[i]}" for i in range(args.shards)])

    for shard in range(args.shards):
        print(f"  {args.job}-{LETTERS[shard]}: "
              f"{len(shard_record[f'{args.job}-{LETTERS[shard]}']['steps']):3d} steps, "
              f"{per_shard[shard]:7.1f} s predicted")
    spread = max(per_shard) - min(per_shard)
    print(f"  imbalance: {spread:.1f} s "
          f"({100 * spread / (sum(per_shard) / len(per_shard)):.1f} % of the mean)")

    if not args.write:
        return 0
    args.workflow.write_text("".join(out))
    write_plan(args.plan, args.workflow, "steps", args.job, shard_record,
               {"source": args.provenance, "costs": str(args.costs),
                "measured_seconds_total": round(sum(per_shard), 2)})
    print(f"rewrote {_display(args.workflow)} and {_display(args.plan)}")
    return 0


def join_lanes(args) -> int:
    """Merge `<job>-a`, `<job>-b`, ... back into `<job>`: the inverse of split_steps.

    This exists because the alternative -- editing two shards by hand until
    they look like one job -- lost the job TAIL, and this workflow spells
    `needs:` there.  The merged lane came back with no DAG edge to `build`,
    every re-split inherited the omission, and `test_parallel_ci_layout` was
    the only thing that noticed.  A join that carries the tail by construction
    makes that class of loss impossible.
    """
    lines = args.workflow.read_text().splitlines(keepends=True)
    spans = job_spans(lines)
    shards = [f"{args.job}-{letter}" for letter in LETTERS]
    present = [shard for shard in shards if shard in spans]
    if len(present) < 2:
        raise SystemExit(f"job {args.job!r} has {len(present)} shard(s) to join "
                         f"(looked for {', '.join(shards[:3])}...)")
    if args.job in spans:
        raise SystemExit(f"job {args.job!r} already exists beside its shards")

    parsed = []
    for shard in present:
        header, blocks, tail = step_blocks(lines, spans[shard])
        names = [step_name(block) for block in blocks]
        if PREAMBLE_END not in names:
            raise SystemExit(f"{shard}: preamble marker {PREAMBLE_END!r} missing")
        cut = names.index(PREAMBLE_END) + 1
        parsed.append((header, blocks[:cut], blocks[cut:], tail))

    def normalise(shard: str, header: list[str]) -> list[str]:
        """The header minus the two things a shard legitimately changes."""
        out = [line.replace(f"  {shard}:", f"  {args.job}:", 1) for line in header]
        for index, line in enumerate(out):
            if line.startswith("    name: "):
                out[index] = re.sub(r" \(shard [a-z]+ of \d+\)\s*$", "\n", line)
                break
        return out

    head, preamble, gates, tail = parsed[0]
    joined = normalise(present[0], head)
    for shard, (other_head, other_preamble, _gates, other_tail) in zip(present[1:], parsed[1:]):
        # The preamble is the lane's identity: a shard that added, dropped or
        # reordered a setup step is not a shard of this lane.  The job id and
        # the shard suffix are the ONLY fields a shard may differ in.
        if normalise(shard, other_head) != joined or other_preamble != preamble \
                or other_tail != tail:
            raise SystemExit(f"{shard}: scaffolding differs from {present[0]}; "
                             "refusing to join two different jobs")

    out = list(lines[:spans[present[0]][0]]) + joined
    out.extend(line for block in preamble for line in block)
    for _head, _preamble, shard_gates, _tail in parsed:
        for block in shard_gates:
            out.extend(_without_generated_note(block))
    out.extend(tail)
    out.extend(lines[spans[present[-1]][1]:])
    for shard in present:
        out = replace_needs(out, shard, [args.job])

    plan = json.loads(args.plan.read_text()) if args.plan.exists() else {"lanes": {}}
    for shard in present:
        plan.get("lanes", {}).pop(shard, None)

    total = sum(len(shard_gates) for _h, _p, shard_gates, _t in parsed)
    print(f"joined {' + '.join(present)} -> {args.job}: {total} gate steps")
    if not args.write:
        return 0
    args.workflow.write_text("".join(out))
    args.plan.write_text(json.dumps(plan, indent=2, sort_keys=True) + "\n")
    print(f"rewrote {_display(args.workflow)} and {_display(args.plan)}")
    return 0


# ── mode: shell ─────────────────────────────────────────────────────────────
GATE_FUNCTION_RE = re.compile(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\(\)\s*\{", re.M)
STEP_RE = re.compile(r"^(?P<indent>\s*)- name: (?P<name>.*\(shard (?P<letter>[a-z]) of (?P<count>\d)\))\s*$")
CONTINUES = re.compile(r"\\\s*$")
TRAP = re.compile(r"^\s*trap\b.*\bERR\b")


def shell_units(body: str) -> tuple[list[str], list[str], list[str], list[str], list[str]]:
    """(head, trap, gate units, close, tail) of a shard step's run body."""
    lines = body.split("\n")
    trap_at = next(i for i, line in enumerate(lines) if TRAP.match(line))
    close_at = next(i for i in range(trap_at + 1, len(lines)) if lines[i].strip() == "}")
    head = lines[:trap_at]
    trap = [lines[trap_at]]
    tail = lines[close_at:]          # the closing brace and everything after it

    units: list[str] = []
    buffer: list[str] = []
    pending: list[str] = []
    for line in lines[trap_at + 1:close_at]:
        stripped = line.strip()
        if not buffer:
            if not stripped:
                pending.append(line)
                continue
            if stripped.startswith("#"):
                pending.append(line)
                continue
        buffer.append(line)
        if not CONTINUES.search(line):
            units.append("\n".join(pending + buffer))
            buffer, pending = [], []
    if buffer:
        units.append("\n".join(pending + buffer))
    return head, trap, units, tail


def unit_key(unit: str) -> str:
    """One gate invocation, spelled as `ci_gate_costs.py` spells it.

    A logical command spans continuation lines, and the cost table keys the
    WHOLE command.  Returning only the first line (the original version) made
    every multi-line gate unmatched against its own measurement, and it also
    made the plan name a gate by a fragment the audit could not find in the
    workflow -- both silent, because the fallback price is a fixed floor.
    """
    lines = [line.strip() for line in unit.splitlines()
             if line.strip() and not line.strip().startswith("#")]
    return " ".join(" ".join(lines).split())

# ── wrapper tail: always fail-closed ────────────────────────────────────────
TEELOG_RE = re.compile(r'tee\s+"?([^"\s|;&]+)"?')
ANNOTATE_RE = re.compile(
    r'ci_annotate_log\.py\s+--title\s+"(?P<title>[^"]*)"\s+"(?P<log>[^"]+)"')


def fail_closed_tail(tail: list[str], function: str) -> list[str]:
    """The shard wrapper after the gate function's closing brace, fail-closed.

    A shard is only evidence if a failing gate can fail the STEP.  The shape
    this splitter originally copied forward was::

        if ! fast_contracts 2>&1 | tee "$RUNNER_TEMP/x.log"; then
          python3 scripts/ci_annotate_log.py --title "..." "$RUNNER_TEMP/x.log"
          exit 1
        fi

    Bash disables `set -e` *and* the ERR trap for the whole body of a function
    invoked as an `if` condition, so the function returned its LAST command's
    status: any earlier failing gate printed its diagnostic and the shard
    stayed green (133 gates could fail that way).  Because the splitter copies
    the tail forward, every re-split re-created the masking shape.

    The tail is therefore normalised here, not preserved: annotate through a
    step-level ERR trap and invoke the function as a plain command, which
    keeps errexit inside it and lets `pipefail` fail the pipeline.
    `check_ci_workflow_shell.py` rejects the conditional shape, so this
    function and that rule are two halves of the same contract.
    """
    text = "".join(tail)
    match = ANNOTATE_RE.search(text)
    title = match.group("title") if match else "Gate output"
    if match:
        log = match.group("log")
    else:
        # No annotation helper in the tail (a synthetic fixture, or a step
        # that only piped through tee): keep ITS tee target rather than
        # inventing a path the rest of the step does not write.
        tee = TEELOG_RE.findall(text)
        log = tee[-1] if tee else '$RUNNER_TEMP/gate.log'
    del text
    brace = tail[0] if tail and tail[0].strip() == "}" else "        }\n"
    if not brace.endswith("\n"):
        brace += "\n"
    return [
        brace,
        "        # The body MUST NOT be invoked from a conditional context\n",
        "        # (`if ! ...`, `||`, `&&`): bash ignores errexit AND the ERR trap\n",
        "        # inside a function called that way, so only the LAST gate could\n",
        "        # ever fail the step.  A plain command keeps both, and the\n",
        "        # step-level trap attaches the transcript before exiting.\n",
        "        annotate_shard() {\n",
        "          rc=$?\n",
        f'          python3 scripts/ci_annotate_log.py --title "{title}" "{log}"\n',
        '          exit "$rc"\n',
        "        }\n",
        "        trap annotate_shard ERR\n",
        f'        {function} 2>&1 | tee "{log}"\n',
    ]


def split_shell(args) -> int:
    text = args.workflow.read_text()
    lines = text.splitlines(keepends=True)

    # Locate every shard step of this lane and the extent of its `run:` block.
    found: list[dict] = []
    index = 0
    while index < len(lines):
        match = STEP_RE.match(lines[index].rstrip("\n"))
        if not match or not match.group("name").startswith(args.lane_prefix):
            index += 1
            continue
        indent = match.group("indent")
        run_at = next(i for i in range(index + 1, len(lines))
                      if re.match(rf"^{re.escape(indent)}  run: \|\s*$", lines[i]))
        end = run_at + 1
        while end < len(lines):
            line = lines[end]
            if line.strip() and not line.startswith(indent + "    "):
                break
            end += 1
        found.append({"name_line": index, "run_at": run_at, "end": end,
                      "indent": indent, "name": match.group("name")})
        index = end
    if len(found) != args.shards:
        raise SystemExit(f"found {len(found)} shard steps of {args.lane_prefix!r}, "
                         f"expected {args.shards}")

    # Each shard step lives in its own JOB, and the plan is keyed by job id:
    # a step name is not a lane.  Keying by step name also made the record
    # un-prunable, so the next split of a different lane deleted these entries
    # as "stale" and the contract shards silently left the audit.
    spans = job_spans(lines)
    jobs = []
    for step in found:
        job = next((name for name, (begin, end) in spans.items()
                    if begin <= step["name_line"] < end), None)
        if job is None:
            raise SystemExit(f"step {step['name']!r} is not inside a job")
        jobs.append(job)
    group = args.group or os.path.commonprefix(jobs).rstrip("-")
    if not group:
        raise SystemExit(f"cannot derive a group name from {jobs}; pass --group")

    parsed = [shell_units("".join(lines[f["run_at"] + 1:f["end"]]).rstrip("\n"))
              for f in found]
    gate_fns = []
    for f in found:
        match = GATE_FUNCTION_RE.search("".join(lines[f["run_at"] + 1:f["end"]]))
        if match is None:
            raise SystemExit("cannot find the gate function name in a shard body")
        gate_fns.append(match.group(1))
    head, trap = parsed[0][0], parsed[0][1]
    for other in parsed[1:]:
        if other[0] != head or other[1] != trap:
            raise SystemExit("shard scaffolding differs between shards; fix by hand first")

    # Every gate unit, in file order, remembered with the shard that owns it now.
    units: list[str] = []
    for _, _, gates, _ in parsed:
        units.extend(gates)

    table = read_costs(args.costs)
    weights, missing = [], []
    for unit in units:
        cost = table.get(unit_key(unit))
        if cost is None:
            missing.append(unit_key(unit))
            cost = MIN_STEP_SECONDS
        weights.append(max(cost, MIN_STEP_SECONDS))
    if missing:
        print(f"warning: {len(missing)} gate(s) have no measurement: "
              + "; ".join(m[:60] for m in missing[:5]), file=sys.stderr)

    assignment = lpt(weights, args.shards)
    per_shard = [sum(w for w, a in zip(weights, assignment) if a == s) for s in range(args.shards)]

    # Rewrite from the bottom up so earlier offsets stay valid.
    record: dict[str, dict] = {}
    positions = {shard: next(p for p, f in enumerate(found)
                             if re.search(rf"\(shard {LETTERS[shard]} of \d+\)", f["name"]))
                 for shard in range(args.shards)}
    for position in sorted(positions.values(), reverse=True):
        shard = next(s for s, p in positions.items() if p == position)
        step = found[position]
        chosen = [unit for i, unit in enumerate(units) if assignment[i] == shard]
        body = head + trap + chosen + fail_closed_tail(parsed[position][3], gate_fns[position])
        # The budget comment above a shard step is part of the record, and it
        # was hand-written once and never updated: it still advertised
        # "191 s / 191 s of 382 s" beside a split that actually ran 234 s /
        # 527 s.  A record that lies in its own comment is worse than no
        # comment, so the splitter owns this block and rewrites it.
        top = step["name_line"]
        while top > 0 and lines[top - 1].startswith(NOTE_INDENT):
            top -= 1
        header = note(
            f"{len(chosen)} of {len(units)} independent gate invocations from the",
            "single step that used to run them all in sequence, assigned by",
            f"longest-processing-time over measured per-gate cost ({per_shard[shard]:.0f} s of",
            f"{sum(per_shard):.0f} s). The shards are a partition:",
            "tests/regression/check_ci_shard_partition.py fails if a gate appears in",
            "both, disappears from both, or reappears in a third step, and",
            "tests/regression/check_ci_lane_plan.py fails if a shard drifts from",
            ".github/ci-lane-plan.json.")
        # Every piece already ends in a newline except the gate units, whose
        # internal newlines are significant for continuations; normalise so a
        # re-split cannot double-space the whole tail on every pass.
        lines[step["run_at"] + 1:step["end"]] = [
            "".join(piece.rstrip("\n") + "\n" for piece in body)]
        lines[top:step["name_line"]] = header
        record[jobs[position]] = {
            "group": group,
            "shards": args.shards,
            "step": step["name"],
            "predicted_gate_seconds": round(per_shard[shard], 2),
            "steps": [{"name": unit_key(unit), "measured_s": round(weight, 2)}
                      for i, (unit, weight) in enumerate(zip(units, weights))
                      if assignment[i] == shard],
        }

    for shard in range(args.shards):
        print(f"  shard {LETTERS[shard]}: "
              f"{sum(1 for a in assignment if a == shard):3d} gates, "
              f"{per_shard[shard]:7.1f} s predicted")
    spread = max(per_shard) - min(per_shard)
    print(f"  imbalance: {spread:.1f} s "
          f"({100 * spread / (sum(per_shard) / len(per_shard)):.1f} % of the mean)")

    if not args.write:
        return 0
    args.workflow.write_text("".join(lines))
    write_plan(args.plan, args.workflow, "shell", group, record,
               {"source": args.provenance, "costs": str(args.costs),
                "measured_seconds_total": round(sum(per_shard), 2)})
    print(f"rewrote {_display(args.workflow)} and {_display(args.plan)}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("steps", "shell", "join"), required=True)
    parser.add_argument("--job", help="job id to split (steps mode) or join (join mode)")
    parser.add_argument("--lane-prefix", help="step-name prefix shared by the shards (shell mode)")
    parser.add_argument("--group",
                        help="record group name (shell mode), default: the shard jobs' common prefix")
    parser.add_argument("--shards", type=int, default=2)
    parser.add_argument("--costs", type=Path,
                        help="measured cost table (not needed by join mode)")
    parser.add_argument("--provenance", default="measured by scripts/ci_gate_costs.py")
    parser.add_argument("--workflow", type=Path, default=WORKFLOW)
    parser.add_argument("--plan", type=Path, default=PLAN)
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    if args.mode in ("steps", "join") and not args.job:
        parser.error(f"--job is required with --mode {args.mode}")
    if args.mode == "shell" and not args.lane_prefix:
        parser.error("--lane-prefix is required with --mode shell")
    if args.mode != "join" and not args.costs:
        parser.error(f"--costs is required with --mode {args.mode}")
    if args.mode == "join":
        return join_lanes(args)
    return split_steps(args) if args.mode == "steps" else split_shell(args)


if __name__ == "__main__":
    sys.exit(main())
