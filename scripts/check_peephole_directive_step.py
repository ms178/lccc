#!/usr/bin/env python3
"""Gate: no peephole walker may treat a directive's *class* as its semantics.

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
both backends share it) wrapped by `steps_over_line` / `LineInfo::is_steppable`
per backend.  A new pass that hand-rolls the skip test re-opens the hole, and no
unit test would notice unless someone thinks to write one; this gate makes the
omission loud instead.

What it enforces
----------------
Any test against `LineKind::Directive` -- `==`, `!=`, or `matches!`, in either
polarity -- must be *answered* in one of two ways:

  * the same condition consults a classifying predicate (`LAW_HELPERS` below),
    so the decision is made about the directive's content and not its class; or
  * the site carries a declared exemption, a `DIRECTIVE-LAW: <reason>` comment on
    the test line or the line above it, where `<reason>` is one of:
        selecting    -- the scan hunts for one named directive (`.cfi_startproc`,
                        `.size`), so it reads directives rather than passing them
        classifying  -- this IS the law (the definition of steppability)
        bails        -- a directive ends the window; the pass loses an
                        optimization rather than crossing opacity

Exemptions are counted and printed, so "PASS" never means "found nothing to
look at", and an exemption whose reason is not in the vocabulary is itself a
finding.

Polarity matters and is no longer trusted to a substring list.  The predecessor
of this gate allowed `!= LineKind::Directive` outright -- which is exactly the
shape of the bypass it existed to catch, because the *negated* class test is what
`if !a && kind != Directive { return }` uses to step past every directive in a
function body.  A whitelist of substrings cannot tell those two apart; a rule
that every site must answer can.

The `--self-test` mode runs the detector over witness snippets, including the
bypass this gate used to miss, so the matcher cannot be quietly neutered.
"""

from __future__ import annotations

import argparse
import re
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Where the line model is walked.  Both backends are in scope: the bug was found
# by grepping, not by reading one file.
TARGETS = [
    ROOT / "src" / "backend" / "x86" / "codegen" / "peephole",
    ROOT / "src" / "backend" / "i686" / "codegen",
]

# A test of a line's *class* against the directive variant.  Requiring the
# `.kind` accessor keeps the classifier's own constructions
# (`line_info(LineKind::Directive, ts)`) and prose out of the report: this gate is
# about deciding by class, and a construction decides nothing.
DIRECTIVE_TEST = re.compile(
    r"\.kind\s*[!=]=\s*LineKind::Directive"
    r"|matches!\s*\(\s*[^)]*\.kind\s*,\s*LineKind::Directive"
)

# A `match kind` arm naming the directive variant.  Its class test sits on the
# scrutinee line, so it needs its own shape: the arm body must either read the
# line's text or consult a law helper, or the arm is passing a directive by class.
ARM_TEST = re.compile(r"^\s*(?:\|\s*)?LineKind::Directive\s*(?:\||=>)")

# A condition that mentions one of these has consulted the law about the
# directive's content, which is the whole requirement.
LAW_HELPERS = (
    "steps_over_line",
    "directive_emits_data",
    "is_alignment_directive",
    "is_debug_marker_directive",
    "benign_directive",
    "inert_directive",
    "is_steppable",
    "is_barrier",
    "directive_is_transparent",
    "opaque_data_directive",
    "is_debug_location",
    "inert_dir",
)

EXEMPTION = re.compile(r"DIRECTIVE-LAW:\s*(selecting|classifying|bails)\b")
REASONS = ("selecting", "classifying", "bails")
# An exemption comment that names a reason outside the vocabulary: a typo or an
# invented excuse must not silence the site it sits on.
EXEMPTION_ANY = re.compile(r"DIRECTIVE-LAW:\s*([A-Za-z_-]+)")


def strip_comments(text: str) -> str:
    """Blank out comment bodies, keeping line structure and code.

    Prose about the law must not be reported as if it tested by class, and a
    `//`-only line can never change behaviour -- so the detector reads a file with
    comments removed while the exemption scan reads the original, where the
    `DIRECTIVE-LAW:` marker still exists.
    """
    out = []
    for line in text.split("\n"):
        stripped = line.lstrip()
        if stripped.startswith("//"):
            out.append("")
            continue
        cut = line.find("//")
        # `//` inside a string literal is not a comment; the tests in this tree
        # carry directive text in string literals, so cut only outside quotes.
        if cut > 0:
            before = line[:cut]
            if before.count('"') % 2 == 0:
                out.append(before)
                continue
        out.append(line)
    return "\n".join(out)


def condition_span(lines: list[str], idx: int) -> str:
    """Text of the condition that line `idx` belongs to, and its body opener.

    Scans up to the enclosing `if`/`while`/`&&` chain and down to the `{`, so a
    test written across three lines is judged as the single decision it is.
    """
    start = idx
    while start > 0:
        prev = lines[start - 1].strip()
        if not prev:
            break
        if any(m in lines[start] for m in ("&&", "||")) or prev.endswith(("&&", "||")):
            start -= 1
            continue
        if prev.startswith(("if ", "while ", "for ", "if(")) or prev.endswith("{"):
            break
        start -= 1
    end = idx
    while end < len(lines) - 1 and "{" not in lines[end]:
        end += 1
    return " ".join(l.strip() for l in lines[start : end + 1])


def exempt_reason(lines: list[str], idx: int) -> tuple[str | None, str | None]:
    """(reason, malformed_reason) for the exemption governing line `idx`.

    Only the test line and the line above it are consulted: an exemption has to
    be *about* the site it excuses, and a comment at the top of a function must
    not silence everything below it.
    """
    probes = [idx]
    probe = idx - 1
    # A `//` comment block immediately above the test belongs to it; a blank line
    # or a statement ends the claim.  Only the last comment line is not enough --
    # the honest exemptions in this tree are two lines long, and reading one line
    # would have made the guard reject comments it exists to accept.
    while probe >= 0 and lines[probe].lstrip().startswith("//"):
        probes.append(probe)
        probe -= 1
    for at in probes:
        line = lines[at]
        m = EXEMPTION.search(line)
        if m:
            return m.group(1), None
        m = EXEMPTION_ANY.search(line)
        if m and m.group(1) not in REASONS:
            return None, m.group(1)
    return None, None


def scan_text(text: str, rel: str, raw: str | None = None) -> tuple[list[str], int]:
    """Findings for one file, plus the count of exemptions that covered them.

    `text` is comment-stripped (what the detector judges); `raw` is the file as
    written, which is where an exemption marker lives.
    """
    lines = text.split("\n")
    marker_lines = (raw if raw is not None else text).split("\n")
    findings: list[str] = []
    exemptions = 0
    for m in DIRECTIVE_TEST.finditer(text):
        lineno = text.count("\n", 0, m.start())
        cond = condition_span(lines, lineno)
        if any(helper in cond for helper in LAW_HELPERS):
            continue  # the directive's content decides, not its class
        negated = "!= LineKind::Directive" in " ".join(cond.split()) or re.search(
            r"!\s*\([^)]*LineKind::Directive", cond
        )
        # A *negated* class test is how a walker steps past directives
        # (`if !a && kind != Directive { return }`), and nothing about the body
        # makes that safe: the line it lands on was chosen by class.  A positive
        # test can also be a conservative bail ("a directive ends this window") or
        # a content decision made one line later, so those are judged on the body
        # -- but only there, and the leniency is not extended to the negated form.
        if not negated:
            # Two lines above, because the idiom in this tree binds the text first
            # (`let t = infos[n].trimmed(store.get(n)); if kind == Directive { … }`).
            window = "\n".join(lines[max(0, lineno - 2) : lineno + 9])
            if any(h in window for h in LAW_HELPERS) or "trimmed(" in window or "store.get(" in window:
                continue
            # The *branch body* decides whether anything is crossed: a real exit
            # (return/break) is conservative and legal, while `continue` in a
            # scanning loop *is* the step-over.  Judged on the body alone, because a
            # window that also contains the code above the test would read an
            # unrelated `continue` as this site stepping past something.
            branch = "\n".join(lines[lineno + 1 : lineno + 5])
            if re.search(r"\b(return|break)\b", branch) and not re.search(
                r"\+=\s*1|next_non_nop", branch
            ):
                continue  # conservative: a directive ends the window, nothing is crossed
        reason, malformed = exempt_reason(marker_lines, lineno)
        if reason:
            exemptions += 1
            continue
        if malformed:
            findings.append(
                f"{rel}:{lineno + 1}: exemption reason {malformed!r} is not one of "
                f"{'/'.join(REASONS)}\n    {lines[lineno].strip()}"
            )
            continue
        findings.append(
            f"{rel}:{lineno + 1}: steps past (or through) a directive by class without "
            f"consulting the law or declaring DIRECTIVE-LAW: <{'|'.join(REASONS)}>\n"
            f"    {marker_lines[lineno].strip()}\n"
            f"    condition: {' '.join(cond.split())[:160]}"
        )
    # `match kind` arms: the class test lives on the scrutinee line, so it is
    # judged on whether the arm reads the directive at all.
    for m in ARM_TEST.finditer(text):
        lineno = text.count("\n", 0, m.start())
        body = "\n".join(lines[lineno : lineno + 6])
        if any(h in body for h in LAW_HELPERS) or "trimmed(" in body or "store.get(" in body:
            continue
        reason, _ = exempt_reason(marker_lines, lineno)
        if reason:
            exemptions += 1
            continue
        findings.append(
            f"{rel}:{lineno + 1}: a Directive match arm neither reads the line's text "
            f"nor consults the law, so it passes a data directive by class"
        )
    return findings, exemptions


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--self-test", action="store_true", help="run the witness corpus and exit")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    # The matcher is run before the matcher: a guard whose regex can be dulled is
    # worse than no guard, because a green run then *means* something it does not.
    # The witnesses include the bypass this gate used to miss, so an allow-list
    # "simplification" that re-admits it fails here first.
    if (rc := self_test()) != 0:
        print("FAIL: the directive-step guard no longer matches its own witnesses", file=sys.stderr)
        return rc

    findings: list[str] = []
    scanned = 0
    exemptions = 0
    for base in TARGETS:
        if not base.is_dir():
            findings.append(f"{base}: missing -- the gate would silently pass")
            continue
        for path in sorted(base.rglob("*.rs")):
            text = path.read_text(encoding="utf-8")
            scanned += len(text.split("\n"))
            found, used = scan_text(
                strip_comments(text), str(path.relative_to(ROOT)), raw=text
            )
            findings.extend(found)
            exemptions += used
    if findings:
        print(
            "FAIL: a directive can emit data; decide about its content "
            "(directive_emits_data/steps_over_line) or declare why this site is not "
            "a step-over\n",
            file=sys.stderr,
        )
        for f in findings:
            print(f"  {f}", file=sys.stderr)
        print(
            f"\n{len(findings)} site(s); see "
            "src/backend/peephole_common.rs (`inert_directive`) and "
            "src/backend/x86/codegen/peephole/passes/helpers.rs for the law.",
            file=sys.stderr,
        )
        return 1
    print(
        f"PASS: every directive class test in the peephole walkers consults the law or "
        f"declares its reason ({scanned} lines scanned, {exemptions} declared exemptions)"
    )
    return 0


# ── witness corpus ────────────────────────────────────────────────────────────

WITNESSES: list[tuple[str, str, bool]] = [
    (
        "the bypass this gate used to allow: a negated class test that steps past data",
        """\
fn next_real(infos: &[LineInfo], len: usize) -> Option<usize> {
    let mut k = 0;
    while k < len {
        if !infos[k].is_nop() && infos[k].kind != LineKind::Directive {
            return Some(k);
        }
        k += 1;
    }
    None
}
""",
        True,
    ),
    (
        "the same walk written through the law",
        """\
fn next_real(infos: &[LineInfo], len: usize) -> Option<usize> {
    let mut k = 0;
    while k < len {
        if !infos[k].is_steppable() {
            return Some(k);
        }
        k += 1;
    }
    None
}
""",
        False,
    ),
    (
        "a positive class test used to step over, from the original bug",
        """\
fn walk(infos: &[LineInfo]) {
    if infos[j].is_nop() || infos[j].kind == LineKind::Directive {
        j += 1;
    }
}
""",
        True,
    ),
    (
        "a class test whose condition consults the content law",
        """\
fn walk(infos: &[LineInfo], store: &LineStore) {
    if infos[j].kind == LineKind::Directive && !directive_emits_data(store.get(j)) {
        j += 1;
    }
}
""",
        False,
    ),
    (
        "a scan for one named directive, declared as such",
        """\
fn find_start(infos: &[LineInfo]) {
    // DIRECTIVE-LAW: selecting -- this loop reads directives to find
    // `.cfi_startproc`; it never passes one of them over.
    if infos[i].is_nop() || infos[i].kind != LineKind::Directive {
        i += 1;
    }
}
""",
        False,
    ),
    (
        "a positive class test whose body decides by content, one line later",
        """\
fn walk(infos: &[LineInfo], store: &LineStore) {
    if infos[j].kind == LineKind::Directive {
        let dl = infos[j].trimmed(store.get(j));
        if is_alignment_directive(dl) {
            j += 1;
        }
    }
}
""",
        False,
    ),
    (
        "a positive class test that advances by class -- data directives crossed",
        """\
fn walk(infos: &[LineInfo]) {
    if infos[k].kind == LineKind::Directive {
        k += 1;
        continue;
    }
}
""",
        True,
    ),
    (
        "a positive class test that ends the window is conservative, and legal",
        """\
fn walk(infos: &[LineInfo]) {
    if infos[q].kind == LineKind::Directive {
        break; // a directive may emit data: do not hoist across it
    }
}
""",
        False,
    ),
    (
        "an invented exemption reason must not silence the site",
        """\
fn walk(infos: &[LineInfo]) {
    // DIRECTIVE-LAW: harmless
    if infos[j].kind == LineKind::Directive {
        j += 1;
    }
}
""",
        True,
    ),
    (
        "an exemption two lines up governs nothing: too far to be about this site",
        """\
fn walk(infos: &[LineInfo]) {
    // DIRECTIVE-LAW: selecting
    let x = 1;
    if infos[j].kind == LineKind::Directive {
        j += 1;
    }
}
""",
        True,
    ),
]


def self_test() -> int:
    checks = 0
    with tempfile.TemporaryDirectory() as tmp:
        for i, (label, source, expect_finding) in enumerate(WITNESSES):
            path = Path(tmp) / f"witness{i}.rs"
            path.write_text(source, encoding="utf-8")
            findings, used = scan_text(source, path.name)
            got = bool(findings)
            checks += 1
            if got != expect_finding:
                verb = "missed" if expect_finding else "flagged"
                print(f"FAIL: self-test {verb} {label!r}\n{findings}", file=sys.stderr)
                return 1
            if not expect_finding and "kind != LineKind::Directive" in source and not used:
                print(f"FAIL: self-test {label!r} passed without consuming an exemption", file=sys.stderr)
                return 1
            checks += 1
    print(f"peephole directive-step guard: {checks} witness checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
