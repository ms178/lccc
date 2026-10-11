#!/usr/bin/env python3
"""Gate: no peephole walker may step over a directive it has not classified.

Why this exists
---------------
Every flag / liveness / memory walk in the x86 and i686 backends used to open
with the same line:

    if infos[j].is_nop() || infos[j].kind == LineKind::Directive { step }

which asserts, without argument, that *no assembler directive can matter*.
Directives that emit data do: in a function body `.byte 0xf5` is `cmc`,
`.byte 0xb8,...` is `mov $imm32,%eax`, `.p2align 4,0xcc` is a run of `int3`,
and `.long .L1-.L2` changes what the neighbouring labels mean.  A walker that
steps over one can move a flag write across a hidden flag read, delete a store
a hidden instruction reads, narrow a `movsq` across a hidden wide read, or drop
a callee-saved save whose restore the hidden code expects.

The law has a single home -- `directive_emits_data` (in `peephole_common`, so
both backends share it) wrapped by `steps_over_line` per backend.  A new pass
that hand-rolls the skip test re-opens the hole, and no unit test would notice
unless someone thinks to write one; this gate makes the omission loud instead.

It is deliberately narrow: it fires on a *step-over* condition (a `Directive`
kind test OR-ed with `is_nop()`), and stays out of the way of scans that hunt
for a specific directive (`.cfi_startproc`, `.size`, jump-table `.long` rows)
or that consult a classifying predicate in the same condition.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Where the line model is walked.  Both backends are in scope: the bug was
# found by grepping, not by reading one file.
TARGETS = [
    ROOT / "src" / "backend" / "x86" / "codegen" / "peephole",
    ROOT / "src" / "backend" / "i686" / "codegen",
]

# A `Directive` test that is being *skipped over* rather than classified.
DIRECTIVE_TEST = re.compile(
    r"kind\s*==\s*LineKind::Directive|matches!\s*\([^)]*LineKind::Directive"
)
STEP_MARKERS = ("if ", "while ", "&&")
# A condition that mentions any of these has consulted the law (or is a
# directive-seeking scan, which is a different question entirely).
ALLOWED = (
    "steps_over_line",
    "directive_emits_data",
    "is_alignment_directive",
    "is_debug_marker_directive",
    "benign_directive",
    "!= LineKind::Directive",
    "!=",  # `kind != LineKind::Directive`: a scan FOR directives, not past them
)


def condition_span(lines: list[str], idx: int) -> str:
    """Text of the `if`/`while` condition that line `idx` belongs to."""
    start = idx
    while start > 0:
        prev = lines[start - 1]
        stripped = prev.strip()
        if not stripped:
            break
        if stripped.endswith(("{", ";", "}", ")")) and not stripped.startswith(
            ("&&", "||")
        ):
            # The previous statement is complete: `idx` opens a new condition
            # unless it is itself the continuation of one.
            if not any(m in lines[idx] for m in ("&&", "||")) and start == idx:
                break
        if any(m in prev for m in STEP_MARKERS) and "{" in stripped + lines[start]:
            break
        if DIRECTIVE_TEST.search(prev):
            pass
        start -= 1
    end = idx
    while end < len(lines):
        if "{" in lines[end] or lines[end].strip().endswith(("{",)):
            break
        end += 1
    return " ".join(l.strip() for l in lines[start : end + 1])


def main() -> int:
    findings: list[str] = []
    scanned = 0
    for base in TARGETS:
        if not base.is_dir():
            findings.append(f"{base}: missing -- the gate would silently pass")
            continue
        for path in sorted(base.rglob("*.rs")):
            text = path.read_text(encoding="utf-8")
            lines = text.split("\n")
            scanned += len(lines)
            for m in DIRECTIVE_TEST.finditer(text):
                lineno = text.count("\n", 0, m.start())
                cond = condition_span(lines, lineno)
                if "is_nop()" not in cond:
                    continue  # not a step-over skip
                if any(a in cond for a in ALLOWED):
                    continue  # the law was consulted in the same condition
                findings.append(
                    f"{path.relative_to(ROOT)}:{lineno + 1}: steps over an "
                    f"unclassified directive\n    {lines[lineno].strip()}"
                )
    if findings:
        print(
            "FAIL: a directive can emit data; step over one only through "
            "steps_over_line()/directive_emits_data()\n",
            file=sys.stderr,
        )
        for f in findings:
            print(f"  {f}", file=sys.stderr)
        print(
            f"\n{len(findings)} site(s); see "
            "src/backend/x86/codegen/peephole/passes/helpers.rs for the law.",
            file=sys.stderr,
        )
        return 1
    print(
        f"PASS: every directive step-over in the peephole walkers consults the "
        f"data-directive law ({scanned} lines scanned)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
