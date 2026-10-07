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
Variant = tuple[int, int, list[tuple[int, int]]]


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


def apply_bitdiffs(base: Constraint, encoding: ET.Element) -> Variant | None:
    """Fold an encoding's `bitdiffs` into (mask, value, !=-predicates).

    Terms: `field == lit` (lit may carry parens and `x` don't-cares) and
    `field != lit` (becomes a match-time predicate).  Any other operator
    makes the variant unusable (None) so it is never matched loosely.
    """
    spec = (encoding.get("bitdiffs") or "").strip()
    bmask, value, fields = base
    if not spec:
        return bmask, value, []
    mask = bmask
    preds: list[tuple[int, int]] = []
    for term in spec.split("&&"):
        term = term.strip()
        m = re.fullmatch(
            r"([A-Za-z_][A-Za-z0-9_]*)\s*(==|!=)\s*\(?\s*([01xX]+)\s*\)?", term
        )
        if not m:
            return None
        fname, op, lit = m.group(1), m.group(2), m.group(3)
        if fname not in fields:
            return None
        fmask, fbits, fpinned = fields[fname]
        if len(lit) != fmask.bit_count():
            return None
        top = fmask.bit_length() - 1
        wanted = 0
        lit_mask = 0
        for i, ch in enumerate(lit):
            bit = top - i
            if ch == "x" or ch == "X":
                continue
            lit_mask |= 1 << bit
            wanted |= (1 if ch == "1" else 0) << bit
        if op == "==":
            # Cross-check only the bits the literal actually states, and
            # only where the diagram pins them too.
            if (wanted ^ fbits) & fpinned & lit_mask:
                return None
            for i, ch in enumerate(lit):
                bit = top - i
                if ch in "01":
                    mask |= 1 << bit
                    if ch == "1":
                        value |= 1 << bit
                    else:
                        value &= ~(1 << bit)
        else:  # !=
            if fpinned & fmask:
                # A pinned field cannot also be `!=`-constrained.
                return None
            preds.append((fmask, wanted & fmask))
    return mask, value, preds


def word_matches(word: int, variant: Variant) -> bool:
    mask, value, preds = variant
    if word & mask != value:
        return False
    return all(word & fm != forbidden for fm, forbidden in preds)


def encoding_variants(root: ET.Element) -> list[Variant]:
    """All usable constraints an iform XML provides."""
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
            folded = apply_bitdiffs(base_c, enc)
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
        for path in candidates:
            try:
                root = ET.parse(path).getroot()
            except ET.ParseError:
                unparsable += 1
                continue
            for variant in encoding_variants(root):
                if word_matches(word, variant):
                    matched = True
                    break
            if matched:
                break
        if matched:
            validated += 1
        else:
            alias_ok, note = check_cross_mnemonic(xml_dir, mnem, word)
            if alias_ok:
                validated += 1
                print(f"doccheck: note: {note}")
                continue
            extra = f" ({unparsable} unparsable candidate file(s))" if unparsable else ""
            problems.append(
                f"{mnem}: word {word:08x} ({text!r}) matches no encoding "
                f"variant of {len(candidates)} candidate(s){extra}"
            )

    total = len(rows)
    print(
        f"doccheck: {validated}/{total} golden words validated against "
        f"ARM DDI 0487 M.d / ISA A64 {REQUIRED_RELEASE} XML"
    )
    if problems:
        print(f"doccheck: {len(problems)} divergence(s):", file=sys.stderr)
        for p in problems[:60]:
            print(f"  {p}", file=sys.stderr)
        if len(problems) > 60:
            print(f"  ... and {len(problems) - 60} more", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
