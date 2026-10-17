#!/usr/bin/env python3
"""Validate dispatcher golden words against the Arm A64 ISA XML package.

Documentation law (pinned 2026-10-07)
--------------------------------------
The authoritative Arm documentation for this tree is

  * Arm Architecture Reference Manual for A-profile architecture,
    ARM DDI 0487, document version **M.d** (latest), and
  * its machine-readable A64 ISA package, release **2026-09**
    (`build-manifest.json` -> `artifact.release_version` =
    ``2026-09_md``).

Every older package is NAK'd: the script refuses to run against a
directory whose build-manifest does not declare a `2026-09` release, so
a stale extract can never silently re-bless old encodings.

What it checks
--------------
Each row of `tests/aarch64/dispatcher-goldens.tsv` is one instruction
word that GNU as 2.47 accepts — one per `encode_instruction()` arm in
`src/backend/arm/assembler/encoder/mod.rs`.  For every row the script

  1. finds the iform XML(s) whose `mnemonic`/`heading` (or, for alias
     spellings such as `ldrw`, the instruction's first token) matches,
  2. reconstructs each encoding variant's constraints from the class
     `<regdiagram>` boxes plus the variant's `<bitdiffs>`, and
  3. checks `word & mask == value` plus every `field != literal`
     predicate.

A golden word that satisfies no variant of its mnemonic is a divergence
between the GAS-2.47-pinned encoding this tree ships and the current Arm
documentation, and fails the run (exit 1).  Unparsable candidate files
and unresolvable rows are failures too — this gate is fail-closed.

Diagram/`bitdiffs` forms handled (all measured in the 2026-09 package):
constant `<c>` bits, `x` don't-cares, parenthesized constants `(1)`,
constraint prose (`!= 11111`) as variable spans, bitdiffs
`field == lit` with optional parens and `x` don't-cares inside the
literal, and `field != lit` predicates.

Usage
-----
    python3 scripts/aarch64_doccheck_2026_09.py \
        [--xml-dir DIR] [--goldens TSV]

Defaults: the documented extract under `/home/user/dl/` and the
in-tree goldens.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

DEFAULT_XML_DIR = Path.home() / (
    ".cache/arm-isa-a64-2026-09/ISA_A64_xml_A_profile-2026-09_md"
)
# Fallback for a workspace where scripts/ensure_arm_a64_xml.sh has not run
# yet but the extract is already on disk (e.g. a manually staged review).
FALLBACK_XML_DIRS = (
    Path("/home/user/dl/isa-a64-2026-09/ISA_A64_xml_A_profile-2026-09_md"),
)
DEFAULT_GOLDENS = Path("tests/aarch64/dispatcher-goldens.tsv")
REQUIRED_RELEASE = "2026-09"

# A constraint tuple: fixed mask/value in absolute word bits, plus the
# named-field table {name: (field_mask, field_ones, pinned_mask)}.
Constraint = tuple[int, int, dict[str, tuple[int, int, int]]]
# A usable variant: mask/value plus `field != literal` predicates.
# A variant is (mask, value, rejects).  `rejects` models a *predicate* over
# fields rather than the single `!=` it used to hold, because the M.d XML uses
# three shapes: `imm12 != 0` (reject on one field), `!(L == 0 && imm3 == 011
# && Rt == 11111)` (hinte.xml: reject only when the conjunction holds), and
# `!(op1 == 000 && op2 IN {00x, 010})` (msr_imm.xml: a term with alternatives).
# Reading the second as three separate `!=`s would reject valid encodings --
# `HINT #1` satisfies L==0 && imm3==011 but not Rt==11111, and a checker that
# over-rejects reports legal instructions as divergent.
#   rejects  : list of patterns, any match forbids the word
#   pattern  : list of terms, all must match to forbid
#   term     : list of (mask, value) pairs, any match satisfies the term
Term = list[tuple[int, int]]
Pattern = list[Term]
Variant = tuple[int, int, list[Pattern]]


# ── cross-mnemonic aliases (explicit, never silent) ──────────────────
#
# The 2026-09 package publishes no scalar UXTW iform or alias page —
# `UXTW <Xd>, <Wn>` appears only as an extend specifier (ADD-extended
# option table) and as part of the SVE zeroing triplet UXTB/UXTH/UXTW
# (grep of heading/docvar/aliaspageid/encodingindex confirms; UXTH and
# UXTB DO have scalar `*_ubfm` alias pages, UXTW has none).
#
# GNU as 2.47 therefore selects the canonical zero-extend form itself:
# `uxtw x1, w0` assembles byte-exactly to `orr w1, wzr, w0`
# (0x2a0003e1), relying on the documented GPR rule that a W-register
# write zeroes the upper 32 bits of the X register.  Rather than drop
# the row or hand-wave it away, we validate the word against the exact
# source document it derives from — ORR (shifted register) in
# `orr_log_shift.xml` — with the alias form pinned: Rn = ZR, shift = LSL,
# N = 0, imm6 = 0.  Any other mnemonic keeps the strict single-mnemonic
# rule above; this table may only grow with a comment proving the
# absence of an iform the same way.
CROSS_MNEMONIC_ALIASES: dict[str, tuple[str, dict[str, str]]] = {
    "uxtw": (
        "orr_log_shift.xml",
        {"Rn": "11111", "shift": "00", "N": "0", "imm6": "000000"},
    ),
}


def fail(msg: str) -> None:
    print(f"doccheck: {msg}", file=sys.stderr)
    sys.exit(1)


def check_manifest(xml_dir: Path) -> None:
    manifest = xml_dir / "build-manifest.json"
    if not manifest.is_file():
        fail(f"missing {manifest} (not an Arm ISA XML extract?)")
    try:
        data = json.loads(manifest.read_text())
    except json.JSONDecodeError as e:
        fail(f"unreadable manifest {manifest}: {e}")
    rel = data.get("artifact", {}).get("release_version", "")
    if REQUIRED_RELEASE not in rel:
        fail(
            f"NAK'd documentation release {rel!r}: this tree pins the "
            f"{REQUIRED_RELEASE} package (ARM DDI 0487 M.d); older "
            "extracts must not be used as an oracle"
        )
    print(f"doccheck: manifest release_version = {rel} (pinned OK)")


# ── regdiagram / bitdiffs parsing ─────────────────────────────────────


def diagram_constraints(diagram: ET.Element) -> Constraint | None:
    """Fixed mask/value of a <regdiagram>, plus named fields.

    Returns None when a box layout cannot be interpreted (fail-closed:
    callers drop the encoding rather than mis-validate).
    """
    mask = 0
    value = 0
    fields: dict[str, tuple[int, int, int]] = {}
    for box in diagram.findall("box"):
        try:
            hibit = int(box.get("hibit", ""))
        except ValueError:
            return None
        width = int(box.get("width", "1"))
        if width < 1 or hibit > 31 or hibit - width + 1 < 0:
            return None
        name = box.get("name")
        cs = box.findall("c")
        pos = hibit
        remaining = width
        idx = 0
        fmask = fbits = fpinned = 0
        while remaining > 0:
            if idx < len(cs):
                c = cs[idx]
                colspan = int(c.get("colspan", "1"))
                text = (c.text or "").strip()
            else:
                # A box with fewer <c> children than its width treats the
                # remainder as one variable span.
                colspan = remaining
                text = ""
            if colspan < 1 or colspan > remaining:
                return None
            low = pos - colspan + 1
            # Normalize `(1)`-style parenthesized constants.
            pm = re.fullmatch(r"\(([01xX]+)\)", text)
            if pm:
                text = pm.group(1)
            is_constraint = text.startswith(("!", "<", ">", "="))
            is_x = re.fullmatch(r"[xX]+", text or "") is not None and len(text) == colspan
            if text == "" or is_x or is_constraint:
                pass  # variable span: nothing pinned
            elif re.fullmatch(r"[01xX]+", text) and len(text) == colspan:
                for i, ch in enumerate(text):
                    bit = pos - i  # first char = highest bit of the span
                    if ch in "01":
                        mask |= 1 << bit
                        fpinned |= 1 << bit
                        if ch == "1":
                            value |= 1 << bit
            else:
                return None
            if name:
                for i in range(colspan):
                    bit = pos - i
                    fmask |= 1 << bit
                    if text and i < len(text) and text[i] == "1" and not is_constraint:
                        fbits |= 1 << bit
            pos -= colspan
            remaining -= colspan
            idx += 1
        if name and fmask:
            fields[name] = (fmask, fbits, fpinned)
    return mask, value, fields


def _split_top(spec: str) -> list[str]:
    """Split on `&&` at depth 0, so `!(a && b)` stays one term.

    The first version used `spec.split("&&")`, which cut *through* the
    negation: `!(L == 0 && imm3 == 011 && Rt == 11111)` became three terms
    `!(L == 0`, `imm3 == 011`, `Rt == 11111)`, none of them parseable, and the
    encoding was dropped as unusable -- a silently unchecked instruction.
    """
    parts: list[str] = []
    depth = 0
    cur = ""
    i = 0
    while i < len(spec):
        ch = spec[i]
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        if depth == 0 and spec.startswith("&&", i):
            parts.append(cur)
            cur = ""
            i += 2
            continue
        if depth == 0 and spec.startswith("||", i):
            # An or-predicate cannot be folded into (mask, value, rejects).
            return []
        cur += ch
        i += 1
    parts.append(cur)
    return [t.strip() for t in parts if t.strip()]


def _lit(term: str, fmask: int) -> "list[tuple[int, int]] | None":
    """(mask, value) pairs for `011`, `(0)`, or `{00x, 010}` against `fmask`."""
    top = fmask.bit_length() - 1
    alts: list[str] = []
    m = re.fullmatch(r"\(\s*([01xX ]+)\s*\)", term)
    if m:
        alts = [m.group(1).replace(" ", "")]
    elif term.startswith("{") and term.endswith("}"):
        alts = [a.strip() for a in term[1:-1].split(",") if a.strip()]
    else:
        alts = [term.replace(" ", "")]
    out: list[tuple[int, int]] = []
    for lit in alts:
        if any(c not in "01xX" for c in lit) or len(lit) != fmask.bit_count():
            return None
        vm = 0
        vv = 0
        for i, ch in enumerate(lit):
            bit = top - i
            if ch in "xX":
                continue
            vm |= 1 << bit
            if ch == "1":
                vv |= 1 << bit
        out.append((vm, vv))
    return out


def apply_bitdiffs(
    base: Constraint, encoding: ET.Element, why: "list[str] | None" = None
) -> "Variant | None":
    """Fold an encoding's `bitdiffs` into (mask, value, rejects).

    Understood shapes: `f == lit` (folds into mask/value, and is cross-checked
    against what the regdiagram pins), `f != lit` (a reject term), and
    `!( t1 && t2 && ... )` where each `ti` is `f == lit` or `f IN {..}` (one
    reject *pattern*: the word is only forbidden when the whole conjunction
    holds).  Anything else is refused with a reason in `why`, because the
    caller drops the variant and an unchecked encoding must not look like a
    passing one -- silently folding a parse failure into "0 variants" is how
    this script used to report 423/423 while three XML forms were simply
    unread (hinte.xml, msr_imm.xml: `!(A && B && C)`, `op2 IN {00x, 010}`).
    """
    def nope(msg: str) -> None:
        if why is not None:
            why.append(msg)

    spec = (encoding.get("bitdiffs") or "").strip()
    bmask, value, fields = base
    if not spec:
        return bmask, value, []
    mask = bmask
    rejects: list[Pattern] = []
    terms = _split_top(spec)
    if not terms:
        # An empty split means the spec was empty (handled above) or carried a
        # shape _split_top refuses (`||`): neither may fall through to the
        # no-constraint return below, because a variant with no rejects matches
        # every word of the iclass and that is how a checker starts certifying
        # encodings it never read.
        nope(f"bitdiff form not understood: {spec[:60]!r}")
        return None
    for term in terms:
        negated = False
        inner = term
        m = re.fullmatch(r"!\s*\((.*)\)", term, re.S)
        if m:
            negated, inner = True, m.group(1).strip()
        conjuncts = [inner] if not negated else _split_top(inner)
        if not conjuncts:
            nope(f"unsupported bitdiff: {term[:60]!r}")
            return None
        pattern: Pattern = []
        for conj in conjuncts:
            cm = re.fullmatch(
                r"([A-Za-z_][A-Za-z0-9_]*)\s*(==|!=)\s*(.*)", conj.strip(), re.S
            )
            if not cm:
                im = re.fullmatch(
                    r"([A-Za-z_][A-Za-z0-9_]*)\s+IN\s+(\{.*\})", conj.strip(), re.S
                )
                if not im:
                    nope(f"unsupported bitdiff term: {conj.strip()[:60]!r}")
                    return None
                # Inside a negation `IN` is one term's alternative list (the
                # msr_imm.xml shape).  A POSITIVE top-level `f IN {a, b}`
                # (gic_sys.xml, 2026-09) is handled below as its own op: the
                # field must take a listed value, which is structurally the
                # same reject form as `!=` — forbid the set's complement.
                fname, lit = im.group(1), im.group(2)
                op = "==" if negated else "IN"
            else:
                fname, op, lit = cm.group(1), cm.group(2), cm.group(3).strip()
            if fname not in fields:
                nope(f"bitdiff names unknown field {fname!r} ({spec[:40]!r})")
                return None
            fmask, fbits, fpinned = fields[fname]
            alts = _lit(lit, fmask)
            if not alts:
                nope(f"bitdiff literal {lit[:24]!r} does not fit {fname} "
                     f"({fmask.bit_count()} bits)")
                return None
            if negated:
                if op == "!=":
                    nope(f"`!=` inside a negation is not understood: {conj[:40]!r}")
                    return None
                pattern.append(alts)
                continue
            if op == "IN":
                # Positive `f IN {a, b}`: reject every field value OUTSIDE the
                # set.  Enumerating the complement needs the whole field's
                # value space, so refuse unenumerably wide fields instead of
                # guessing (fail closed, like every other refusal here).
                if fpinned & fmask:
                    nope(f"{fname} is pinned by the diagram yet `IN`-constrained")
                    return None
                nbits = fmask.bit_count()
                if nbits > 8:
                    nope(f"positive `IN` on a {nbits}-bit field is not "
                         f"enumerable: {conj.strip()[:40]!r}")
                    return None
                positions = [i for i in range(fmask.bit_length()) if fmask >> i & 1]
                comp: "list[tuple[int, int]]" = []
                for fv in range(1 << nbits):
                    vv = 0
                    for j, pos in enumerate(positions):
                        if fv >> j & 1:
                            vv |= 1 << pos
                    if not any((vv & am) == av for am, av in alts):
                        comp.append((fmask, vv))
                # An `IN` covering the whole value space constrains nothing.
                if comp:
                    pattern.append(comp)
                continue
            if op == "!=":
                if fpinned & fmask:
                    nope(f"{fname} is pinned by the diagram yet `!=`-constrained")
                    return None
                pattern.append(alts)
                continue
            # Positive `==`: cross-check against the diagram, then fold in.
            for vm, vv in alts:
                if (vv ^ fbits) & fpinned & vm:
                    nope(f"bitdiff {fname}=={lit[:16]!r} contradicts the regdiagram")
                    return None
            if len(alts) != 1:
                nope(f"`IN` on a positive term is not foldable: {conj[:40]!r}")
                return None
            vm, vv = alts[0]
            mask |= vm
            value = (value & ~vm) | (vv & vm)
        if negated or pattern:
            rejects.append(pattern)
    return mask, value, rejects


def word_matches(word: int, variant: Variant) -> bool:
    mask, value, rejects = variant
    if word & mask != value:
        return False
    for pattern in rejects:
        # A pattern forbids the word only when EVERY term is satisfied, and a
        # term is satisfied by ANY of its alternatives (`IN {00x, 010}`).
        if all(any(word & m == v for m, v in term) for term in pattern):
            return False
    return True


def encoding_variants(root: ET.Element, why: "list[str] | None" = None) -> list[Variant]:
    """All usable constraints an iform XML provides.

    `why` collects every bitdiff form that had to be refused, so a caller can
    tell "no variant matched" apart from "no variant could be read".
    """
    out: list[Variant] = []
    for iclass in root.iter("iclass"):
        class_diagram = iclass.find("regdiagram")
        class_c = diagram_constraints(class_diagram) if class_diagram is not None else None
        encodings = iclass.findall("encoding")
        if not encodings and class_c is not None:
            out.append((class_c[0], class_c[1], []))
        for enc in encodings:
            diagram = enc.find("regdiagram")
            base_c = diagram_constraints(diagram) if diagram is not None else class_c
            if base_c is None:
                continue
            folded = apply_bitdiffs(base_c, enc, why)
            if folded is not None:
                out.append(folded)
    return out


def fields_of(root: ET.Element) -> "dict[str, tuple[int, int, int]]":
    """Merged named-field table of every class diagram in the file."""
    out: "dict[str, tuple[int, int, int]]" = {}
    for iclass in root.iter("iclass"):
        diagram = iclass.find("regdiagram")
        if diagram is None:
            continue
        c = diagram_constraints(diagram)
        if c is not None:
            out.update(c[2])
    return out


def check_cross_mnemonic(
    xml_dir: Path, mnem: str, word: int
) -> "tuple[bool, str]":
    """Validate `word` via the explicit alias table; (ok, note)."""
    entry = CROSS_MNEMONIC_ALIASES.get(mnem.lower())
    if entry is None:
        return False, ""
    target, pins = entry
    path = xml_dir / target
    try:
        root = ET.parse(path).getroot()
    except (ET.ParseError, OSError):
        return False, ""
    fields = fields_of(root)
    for variant in encoding_variants(root):
        if not word_matches(word, variant):
            continue
        ok = True
        for fname, lit in pins.items():
            if fname not in fields:
                ok = False
                break
            fmask, _, fpinned = fields[fname]
            if len(lit) != fmask.bit_count():
                ok = False
                break
            top = fmask.bit_length() - 1
            wanted = 0
            for i, ch in enumerate(lit):
                if ch == "1":
                    wanted |= 1 << (top - i)
            if word & fmask != wanted or fpinned & ~fmask:
                ok = False
                break
        if ok:
            note = (
                f"{mnem}: validated via cross-mnemonic alias "
                f"({target} with {pins}) — the Arm XML 2026-09 package "
                "publishes no scalar UXTW iform"
            )
            return True, note
    return False, ""


# ── corpus indexing ───────────────────────────────────────────────────


def index_by_mnemonic(xml_dir: Path) -> "dict[str, list[Path]]":
    index: "dict[str, list[Path]]" = {}
    for path in sorted(xml_dir.glob("*.xml")):
        if path.name == "index.xml":
            continue
        try:
            text = path.read_text(errors="replace")
        except OSError:
            continue
        mnems = set()
        # The mnemonic docvar is authoritative; some files place their
        # <docvars> past the first few kilobytes, so scan the whole file.
        for m in re.finditer(r'<docvar key="mnemonic" value="([^"]+)"', text):
            mnems.add(m.group(1).upper())
        m = re.search(r"<heading>([^<]+)</heading>", text)
        if m:
            heading = m.group(1).strip().upper()
            mnems.add(heading)
            # Headings enumerate aliases: "CAS, CASA, CASAL, CASL" or
            # "CMP (immediate)" -> index each comma-separated base token.
            for tok in heading.split(","):
                tok = tok.strip()
                tok = tok.split(" (")[0].strip()
                tok = re.split(r"[—–-]+$", tok)[0].strip()
                if re.fullmatch(r"[A-Z][A-Z0-9]*", tok or ""):
                    mnems.add(tok)
        for mnem in mnems:
            if mnem:
                index.setdefault(mnem, []).append(path)
    return index


def load_goldens(path: Path) -> "list[tuple[str, str, int]]":
    rows = []
    for line in path.read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 3:
            fail(f"malformed golden row: {line!r}")
        mnem, text, word = parts
        # The TSV stores the encoding as GNU-as-style little-endian hex
        # (byte sequence as written to the object file), not as a u32
        # literal: `00a135d4` is the word 0xd435a100 on disk.
        rows.append((mnem, text, int.from_bytes(bytes.fromhex(word), "little")))
    return rows


def selftest() -> int:
    """The bitdiff parser's contract, without XML or cross tools.

    This parser rotted quietly for a release because nothing pinned it: an
    unreadable form returned None, `encoding_variants` dropped the variant, and
    the run still printed a satisfied count.  These cases are the three shapes
    real M.d XML uses (hinte.xml, msr_imm.xml) plus the refusal paths, so the
    next time the XML grows a form we cannot read the failure is a red
    `--selftest`, not a quiet pass.
    """
    # fields: L = bit 21, imm3 = bits 20:18, Rt = 4:0, imm12 = 21:10, op1/op2.
    f_l = (1 << 21, 0, 0)
    f_imm3 = (0b111 << 18, 0, 0)
    f_rt = (0b11111, 0, 0)
    f_imm12 = (0xFFF << 10, 0, 0)
    f_op1 = (0b111 << 16, 0, 0)
    f_op2 = (0b111 << 19, 0, 0)
    f_crm = (0b1111 << 8, 0, 0)
    fields = {"L": f_l, "imm3": f_imm3, "Rt": f_rt, "imm12": f_imm12,
              "op1": f_op1, "op2": f_op2, "CRm": f_crm}
    base: Constraint = (0, 0, fields)

    def enc(spec: str, why: "list[str] | None" = None):
        e = ET.Element("encoding", {"bitdiffs": spec})
        return apply_bitdiffs(base, e, why)

    bad: "list[str]" = []

    def expect(cond: bool, msg: str) -> None:
        if not cond:
            bad.append(msg)

    # !(A && B && C): reject the conjunction, never the individual terms.
    v = enc("!(L == 0 && imm3 == 011 && Rt == 11111)")
    expect(v is not None, "negated conjunction must be understood")
    if v is not None:
        mask, value, rejects = v
        expect(mask == 0 and value == 0,
               f"a negation must not fold into mask/value, got {mask:08x}/{value:08x}")
        expect(len(rejects) == 1 and len(rejects[0]) == 3,
               f"want one pattern of three terms, got {rejects}")
        hint0 = (0b011 << 18) | 0b11111          # L=0, imm3=011, Rt=xzr -> HINT #0
        hint1 = (hint0 & ~0b11111) | 0b00001     # same, but Rt != xzr
        expect(not word_matches(hint0, v), "the forbidden conjunction must be rejected")
        expect(word_matches(hint1, v), "one term differing must stay legal")

    # !(op1 == 000 && op2 IN {00x, 010}): a term with alternatives.
    v = enc("!(op1 == 000 && op2 IN {00x, 010})")
    expect(v is not None, "`IN {{...}}` inside a negation must be understood")
    if v is not None:
        a = (0b000 << 16) | (0b001 << 19)   # op1=000, op2=00x -> forbidden
        b = (0b000 << 16) | (0b011 << 19)   # op1=000, op2=011 -> legal
        c = (0b101 << 16) | (0b000 << 19)   # op1!=000          -> legal
        expect(not word_matches(a, v), "op2=00x with op1=000 must be forbidden")
        expect(word_matches(b, v), "op2=011 with op1=000 must be legal")
        expect(word_matches(c, v), "op1!=000 must be legal whatever op2 is")

    # Positive == folds into mask/value; != stays a one-term reject.
    v = enc("imm3 == 011")
    expect(v is not None and v[0] == 0b111 << 18 and v[1] == 0b011 << 18,
           f"positive == must fold, got {v}")
    # Literals are field-width: the M.d XML spells `imm3 != 100`, never
    # `imm12 != 0`, and a short literal is refused as a width mismatch below.
    v = enc("imm3 != 100")
    if v is not None:
        expect(not word_matches(0b100 << 18, v), "imm3 == 100 must be forbidden")
        expect(word_matches(0b011 << 18, v), "imm3 == 011 must be legal")
    else:
        bad.append("`f != lit` must stay understood (it is 62 rows of the corpus)")
    expect(enc("imm12 != 0") is None,
           "a literal narrower than its field must be refused, not zero-extended")

    # Positive `CRm IN {0001, 0010}` (gic_sys.xml, 2026-09 package): the field
    # must take a listed value; the complement is rejected, nothing folds.
    v = enc("CRm IN {0001, 0010}")
    expect(v is not None, "positive `IN {{...}}` must be understood")
    if v is not None:
        mask, value, rejects = v
        expect(mask == 0 and value == 0,
               f"positive `IN` must not fold into mask/value, got {mask:08x}/{value:08x}")
        expect(not word_matches(0b0000 << 8, v), "CRm == 0000 must be forbidden")
        expect(word_matches(0b0001 << 8, v), "CRm == 0001 must be legal")
        expect(word_matches(0b0010 << 8, v), "CRm == 0010 must be legal")
        expect(not word_matches(0b0011 << 8, v), "CRm == 0011 must be forbidden")
        expect(not word_matches(0b1111 << 8, v), "CRm == 1111 must be forbidden")
    why: "list[str]" = []
    expect(enc("imm12 IN {000000000000}", why) is None,
           "positive `IN` on an unenumerably wide field must be refused")
    expect(why and "not enumerable" in why[-1],
           f"reason must say the field is too wide, got {why}")

    # Refusals must be recorded, not silent.
    why = []
    expect(enc("L == 0 || L == 1", why) is None, "or-predicates must be refused")
    expect(
        why and "not understood" in why[-1] and "||" in why[-1],
        f"a refusal must record a reason naming the spec, got {why}",
    )
    why = []
    expect(enc("nosuchfield == 0", why) is None, "an unknown field must be refused")
    expect(why and "unknown field" in why[-1], f"reason must name the field, got {why}")
    why = []
    expect(enc("imm3 == 0111", why) is None, "a literal wider than the field must be refused")
    expect(why and "does not fit" in why[-1], f"reason must say why, got {why}")

    for m in bad:
        print(f"doccheck --selftest: FAIL: {m}", file=sys.stderr)
    print(f"doccheck --selftest: {len(bad)} failure(s)")
    return 1 if bad else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--xml-dir", type=Path, default=DEFAULT_XML_DIR)
    ap.add_argument("--goldens", type=Path, default=DEFAULT_GOLDENS)
    args = ap.parse_args()

    xml_dir = args.xml_dir
    if not xml_dir.is_dir() and args.xml_dir == DEFAULT_XML_DIR:
        # Only the DEFAULT may fall back (to a staged extract); an
        # explicitly requested path is exactly what must exist.
        for alt in FALLBACK_XML_DIRS:
            if alt.is_dir():
                xml_dir = alt
                break
    if not xml_dir.is_dir():
        fail(
            f"XML directory not found: {args.xml_dir} "
            "(run scripts/ensure_arm_a64_xml.sh to provision the pinned "
            "2026-09 package)"
        )
    check_manifest(xml_dir)

    goldens = args.goldens
    if not goldens.is_file():
        goldens = Path(__file__).resolve().parent.parent / args.goldens
    if not goldens.is_file():
        fail(f"goldens not found: {args.goldens}")
    rows = load_goldens(goldens)
    index = index_by_mnemonic(xml_dir)

    validated = 0
    problems: "list[str]" = []
    unread_total = 0
    for mnem, text, word in rows:
        candidates = index.get(mnem.upper(), [])
        if not candidates:
            # Alias spellings our dispatcher uses (`ldrw`, `addhn2`, ...)
            # fall back to the instruction text's first token.
            tok = text.split()[0].upper().rstrip(",") if text else ""
            candidates = index.get(tok, [])
        if not candidates:
            problems.append(f"{mnem}: no iform XML for {text!r} (word {word:08x})")
            continue
        matched = False
        unparsable = 0
        unparsed: "list[str]" = []
        # (counted below whether or not this row ends up matching)
        for path in candidates:
            try:
                root = ET.parse(path).getroot()
            except ET.ParseError:
                unparsable += 1
                continue
            for variant in encoding_variants(root, unparsed):
                if word_matches(word, variant):
                    matched = True
                    break
            if matched:
                break
        unread_total += len(unparsed)
        if matched:
            validated += 1
        else:
            alias_ok, note = check_cross_mnemonic(xml_dir, mnem, word)
            if alias_ok:
                validated += 1
                print(f"doccheck: note: {note}")
                continue
            extra = f" ({unparsable} unparsable candidate file(s))" if unparsable else ""
            if unparsed:
                uniq = sorted(set(unparsed))
                extra += (f"; {len(unparsed)} bitdiff form(s) this script could not "
                          f"read, so those encodings were NOT checked: "
                          + "; ".join(uniq[:4]))
            problems.append(
                f"{mnem}: word {word:08x} ({text!r}) matches no encoding "
                f"variant of {len(candidates)} candidate(s){extra}"
            )

    total = len(rows)
    print(
        f"doccheck: {validated}/{total} golden words validated against "
        f"ARM DDI 0487 M.d / ISA A64 {REQUIRED_RELEASE} XML"
    )
    if unread_total:
        # Fail closed.  A row can still "validate" through another variant of
        # the same iclass while the encoding whose bitdiff we could not read is
        # skipped, and that is a *looser* check that reports a pass.  The only
        # honest answer is to teach the parser, not to keep the tally quiet.
        print(
            f"doccheck: FAIL - {unread_total} bitdiff form(s) on the candidate "
            f"paths of these goldens could not be read; the affected encodings "
            f"were not checked at all",
            file=sys.stderr,
        )
        return 1
    if problems:
        print(f"doccheck: {len(problems)} divergence(s):", file=sys.stderr)
        for p in problems[:60]:
            print(f"  {p}", file=sys.stderr)
        if len(problems) > 60:
            print(f"  ... and {len(problems) - 60} more", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    if "--selftest" in sys.argv:
        sys.exit(selftest())
    sys.exit(main())
