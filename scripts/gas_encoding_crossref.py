#!/usr/bin/env python3
"""Cross-reference both encoder backends against GNU as 2.47's own instruction table.

Why a second reference
----------------------
`sdm_isa_crossref.py` checks the encoder against the Intel SDM through XED's
ISA description.  The SDM is the authority on what an encoding *means*; it is
not an authority on what the AT&T mnemonic for it is, and it does not tell us
which spellings the assembler we are diff-tested against actually accepts.  The
differential harness compares LCCC against `as` 2.47, so the exact question
"would `as` 2.47 assemble this mnemonic to these bytes?" needs `as` 2.47's own
table.

That table is `opcodes/i386-opc.tbl` in binutils-gdb at `binutils-2_47-branch`.
It is not a mnemonic list: every row is
`mnemonic, <opcode expression>, <cpu>, <attributes>, { operands }`, where the
opcode expression packs the legacy prefixes, the opcode-map escapes, the opcode
byte and the `/digit` ModR/M extension into one integer, and where rows are
generated from *templates* that expand into the families (`<alu2>` -> add, or,
adc, sbb, and, sub, xor; `<sd>` -> the single/double pairs; `<bw>` -> the
byte/word pairs; ...).  Parsing it without expanding the templates loses the
largest families, so this tool expands them.  Two details of that field
matter and are pinned by ``tests/corpus/test_gas_encoding_crossref.py``: the
*last* byte of the opcode field is the opcode, so a lone `f2`/`f3`/`66` is an
opcode and not a prefix (`andn`, `blsi`, `blsr`, `blsmsk` are exactly this
shape), and the APX `EVexMap4..7` rows carry only a prefix there and are left
to the SDM gate, which models those maps exactly.

What it answers, all offline and from a vendored snapshot
---------------------------------------------------------
* ``--verify``   every declarative encoder row is a legal `as` 2.47 encoding of
                 that mnemonic (LCCC-EXTRA pinpoints a wrong opcode/prefix/digit),
                 or an invented mnemonic (LCCC-UNKNOWN).
* ``--missing``  every (mnemonic, encoding) `as` 2.47 accepts that no
                 *declarative row* models, grouped by mnemonic.  Read "what a row
                 cannot see" below before treating the count as a worklist.
* ``--coverage`` write the mnemonics with no declarative row at all to JSON.
* ``--absent``   the same set as a flat mnemonic list.
* ``--triage``   with ``--absent``: split that list by whether the mnemonic is
                 spelled *anywhere* in the backend's sources.  Neither half is a
                 verdict: a mention can be a comment, a deny-list entry or a
                 family prefix that does not cover the spelling, and a family
                 that composes its mnemonic (`format!`, `concat!`) leaves no
                 literal at all, so a supported instruction can land in
                 UNMENTIONED.  It is a reading aid for the two counts, run
                 interactively (one full source scan per name), never in CI.
* ``--summary``  counts per family/map, and the template-expansion statistics.

What a row cannot see
---------------------
The model sees only a backend's *declarative* rows, and the 64-bit backend
encodes most instructions in code: `encode_alu` derives the opcode
arithmetically (`0x01 + op * 8`, `encoder/gp_integer.rs`), the `jcc`/`cmov*`/
`setcc` families dispatch on a suffix match, and the VEX/EVEX emitters share
helpers, so none of them has a row.  ``--missing``/``--absent`` therefore count
those instructions as gaps although the assembler encodes them: with
``--backend all`` the current tree reports `add`, `call`, `cmovle` and the whole
`jcc` family under ``--absent`` and `add 01` under ``--missing``, while
`encode_alu` computes that `01` and the other spellings appear as literals in
`encoder/*.rs`.  The two counts are a *declarative-row* census,
not an ISA-coverage census, and the sound direction is ``--verify`` — it asks
whether each row that does exist is a legal encoding.  Making the other two
directions a real worklist means teaching the model the code-driven families
(`backlog.md` MB-45), not reading their counts as missing instructions.

`--backend {all,i686,x86}` selects the encoder sources.  The 64-bit backend is
not a superset of the 32-bit one — x87 integer memory arithmetic, `fcmov*` and
`fucom*` live only in `i686` — so `--backend all` is the default question worth
asking before a release.

Provenance
----------
The vendored snapshot ``scripts/isa/gas-2.47.json`` is *derived* from the table
above; ``scripts/isa/gas-2.47.manifest.json`` records the exact upstream path,
ref, blob digest and the command that regenerates it.  As with the SDM
catalogue, the snapshot is what makes the gate hermetic: a check that needs the
network is a check that gets skipped.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from itertools import product
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ISA_DIR = REPO / "scripts" / "isa"
SNAPSHOT = ISA_DIR / "gas-2.47.json"
MANIFEST = ISA_DIR / "gas-2.47.manifest.json"

UPSTREAM_REPO = "RTEMS/sourceware-mirror-binutils-gdb"
UPSTREAM_PATH = "opcodes/i386-opc.tbl"
UPSTREAM_REF = "binutils-2_47-branch"

LEGACY_PREFIXES = (0x66, 0xF2, 0xF3)

PP_FOR_BYTE = {0x66: "66", 0xF2: "f2", 0xF3: "f3"}

# Opcodes whose store direction is 0x10 above the load form (SDM Vol. 2B).
SSE_INT_MOVE = {0x6E, 0x6F, 0x7E, 0x7F}


# --------------------------------------------------------------------------
# A very small expression evaluator for the opcode field.
#
# The field is C-like arithmetic over byte-packed opcodes: `0x83/<alu2:opc>`,
# `0x04 | (<alu2:opc> << 3)`, `0x660f3a0c | <sd:opc>`.  Supporting `|`, `<<`,
# `+`, parentheses and integer literals covers every row in the table; anything
# else raises and the row is reported instead of being guessed at.
# --------------------------------------------------------------------------
class ExprError(ValueError):
    pass


_TOKEN = re.compile(r"\s*(0x[0-9A-Fa-f]+|\d+|<<|\||\+|\*|\(|\))")


def _tokens(text: str) -> list[str]:
    out, pos = [], 0
    while pos < len(text):
        if text[pos].isspace():
            pos += 1
            continue
        m = _TOKEN.match(text, pos)
        if not m:
            raise ExprError(f"cannot tokenise {text!r} at {pos}")
        out.append(m.group(1))
        pos = m.end()
    return out


def eval_expr(text: str) -> int:
    """Evaluate an opcode expression with C precedence: `|` < `+` < `*` < `<<`."""
    toks = _tokens(text)
    if not toks:
        raise ExprError("empty expression")
    pos = 0

    def peek() -> str | None:
        return toks[pos] if pos < len(toks) else None

    def primary() -> int:
        nonlocal pos
        tok = peek()
        if tok is None:
            raise ExprError("unexpected end")
        if tok == "(":
            pos += 1
            val = bitor()
            if peek() != ")":
                raise ExprError("unbalanced parentheses")
            pos += 1
            return val
        pos += 1
        try:
            return int(tok, 0)
        except ValueError as exc:  # pragma: no cover - token regex prevents it
            raise ExprError(f"not a number: {tok}") from exc

    def shift() -> int:
        nonlocal pos
        val = primary()
        while peek() == "<<":
            pos += 1
            val <<= primary()
        return val

    def mul() -> int:
        nonlocal pos
        val = shift()
        while peek() == "*":
            pos += 1
            val *= shift()
        return val

    def add() -> int:
        nonlocal pos
        val = mul()
        while peek() == "+":
            pos += 1
            val += mul()
        return val

    def bitor() -> int:
        nonlocal pos
        val = add()
        while peek() == "|":
            pos += 1
            val |= add()
        return val

    val = bitor()
    if pos != len(toks):
        raise ExprError(f"trailing tokens in {text!r}")
    return val


def opcode_bytes(value: int) -> list[int]:
    """`0x660f3a0c` -> [0x66, 0x0f, 0x3a, 0x0c]; `0xf0f` -> [0x0f, 0x0f]."""
    if value < 0:
        raise ExprError("negative opcode")
    width = max(1, (value.bit_length() + 7) // 8)
    if value < 0x100:
        width = 1
    return [(value >> (8 * (width - 1 - i))) & 0xFF for i in range(width)]


# --------------------------------------------------------------------------
# Table parsing: plain rows, template declarations, template instantiations.
# --------------------------------------------------------------------------
DECL_NAME = re.compile(r"^<([A-Za-z_][A-Za-z0-9_]*):")
TEMPLATE_HOLE = re.compile(r"<[A-Za-z_]")


def _split_top(text: str, sep: str = ",") -> list[str]:
    """Split on `sep` outside `<>`, `{}` and parentheses."""
    out, depth_angle, depth_brace, depth_paren, start = [], 0, 0, 0, 0
    for i, ch in enumerate(text):
        # `<` opens a template reference only when it is followed by a name;
        # the opcode language also uses `<<` (shift), and counting those two
        # as brackets silently swallows every following field separator.
        if ch == "<" and i + 1 < len(text) and (text[i + 1].isalpha() or text[i + 1] == "_"):
            depth_angle += 1
        elif ch == ">" and depth_angle:
            depth_angle -= 1
        elif ch == "{":
            depth_brace += 1
        elif ch == "}":
            depth_brace -= 1
        elif ch == "(":
            depth_paren += 1
        elif ch == ")":
            depth_paren -= 1
        elif ch == sep and depth_angle == 0 and depth_brace == 0 and depth_paren == 0:
            out.append(text[start:i])
            start = i + 1
    out.append(text[start:])
    return out


def _logical_lines(text: str) -> list[str]:
    """Join lines continued with a trailing `+` (the table's template blocks)."""
    out, buf = [], ""
    for raw in text.splitlines():
        if not raw.strip():
            continue
        line = raw.rstrip()
        stripped = line.strip()
        if stripped.startswith("//") or stripped.startswith("#"):
            continue
        continued = stripped.endswith("+")
        if continued:
            # Drop only the continuation marker: the trailing comma is part of
            # the list being continued, not part of the marker.
            stripped = stripped[:-1].rstrip()
        buf = f"{buf} {stripped}".strip() if buf else stripped
        if not continued:
            out.append(buf)
            buf = ""
    if buf:
        out.append(buf)
    return out


def parse_table(text: str) -> tuple[list[dict], dict[str, list[dict]], dict[str, int]]:
    """Return (rows, templates, stats).  Rows are fully expanded."""
    templates: dict[str, list[dict]] = {}
    raw_rows: list[str] = []
    stats = {"declarations": 0, "instantiations": 0, "literal_rows": 0, "skipped": 0}

    for line in _logical_lines(text):
        m = DECL_NAME.match(line)
        if m and line.endswith(">"):
            name = m.group(1)
            body = line[line.index(":") + 1 : line.rindex(">")]
            # The declaration is `<NAME:field:field, inst, inst, ...>`; the first
            # element is the field list, whose element 0 is the implicit name.
            parts = _split_top(body)
            fields = ["name"] + [f.strip() for f in parts[0].split(":") if f.strip()]
            instances = []
            for item in parts[1:]:
                if not item.strip():
                    continue
                vals = [v.strip() for v in item.split(":")]
                inst = {f: (vals[i] if i < len(vals) else "") for i, f in enumerate(fields)}
                instances.append(inst)
            templates[name] = instances
            stats["declarations"] += 1
            continue
        if line.startswith("{"):
            # `{<pseudopfx>}, ...` builds the lock/rep pseudo-prefix rows, which
            # have no opcode of their own beyond the prefix byte.
            stats["skipped"] += 1
            continue
        raw_rows.append(line)

    rows: list[dict] = []
    for line in raw_rows:
        refs = list(dict.fromkeys(re.findall(r"<([A-Za-z_][A-Za-z0-9_]*)", line)))
        if refs:
            stats["instantiations"] += 1
            unknown = [r for r in refs if r not in templates]
            if unknown:
                stats["skipped"] += 1
                continue
            choices = [templates[r] for r in refs]
            for combo in product(*choices):
                expansion = dict(zip(refs, combo))
                expanded = _substitute(line, expansion)
                parsed = _parse_row(expanded)
                if not parsed:
                    stats["skipped"] += 1
                    continue
                rows.extend(parsed)
        else:
            stats["literal_rows"] += 1
            parsed = _parse_row(line)
            if not parsed:
                stats["skipped"] += 1
                continue
            rows.extend(parsed)
    return rows, templates, stats


def _substitute(text: str, expansion: dict[str, dict]) -> str:
    def repl(m: re.Match) -> str:
        name, field = m.group(1), m.group(2)
        inst = expansion[name]
        if field is None:
            # A first field that starts with `$` is a *selector* (`$avx`, `$sse`,
            # `$apx`): it chooses between alternative cpu/attribute spellings of
            # the same mnemonic and contributes no text to it.
            value = inst.get("name", "")
            return "" if value.startswith("$") else value
        return inst.get(field, "")

    return re.sub(r"<([A-Za-z_][A-Za-z0-9_]*)(?::([A-Za-z_][A-Za-z0-9_]*))?>", repl, text)


def _parse_row(line: str) -> dict | None:
    parts = _split_top(line)
    if len(parts) < 4:
        return None
    mnemonic = parts[0].strip()
    if not mnemonic or not re.fullmatch(r"[a-z][a-z0-9]*", mnemonic):
        return None
    opcode_field = parts[1].strip()
    attrs = parts[3].strip()
    # Pseudo rows and anything still carrying an unexpanded template hole.
    # `<` alone is not a hole: `<<` is the shift operator of the opcode language.
    if TEMPLATE_HOLE.search(opcode_field) or TEMPLATE_HOLE.search(attrs):
        return None
    byte_expr, _, ext_expr = opcode_field.partition("/")
    try:
        value = eval_expr(byte_expr)
    except ExprError:
        return None
    ext = None
    if ext_expr:
        try:
            ext = eval_expr(ext_expr)
        except ExprError:
            # 3DNow! rows carry an immediate suffix (`0xf0f/0x9e`); the suffix is
            # an opcode byte, not a ModR/M digit, so it is not comparable here.
            return None
    built = _canonical(value, ext, attrs)
    if built is None:
        return None
    enc, fams = built
    out = []
    for fam in fams:
        one = dict(enc, family=fam)
        one["mnemonic"] = mnemonic
        one["directions"] = _directions(one, attrs)
        out.extend(one["directions"])
    return out


def _canonical(value: int, ext: int | None, attrs: str) -> dict | None:
    try:
        arr = opcode_bytes(value)
    except ExprError:
        return None
    # A row carrying both `Vex` and `EVex` is dual-encoded: `as` emits the VEX
    # form for 128/256-bit operands and the EVEX form where a mask or zmm
    # register requires it.  Both have to be admitted here.
    if "EVex" in attrs and "Vex" in attrs:
        family = "vexevex"
    elif "EVex" in attrs:
        family = "evex"
    elif "Vex" in attrs:
        family = "vex"
    else:
        family = "legacy"
    w = None
    if "VexW1" in attrs:
        w = 1
    elif "VexW0" in attrs:
        w = 0
    map_ = None
    if "Space0F38" in attrs:
        map_ = "0f38"
    elif "Space0F3A" in attrs:
        map_ = "0f3a"
    elif "Space0F" in attrs:
        map_ = "0f"
    pp = "none"
    if family == "legacy":
        seen_66 = False
        while arr and arr[0] in LEGACY_PREFIXES:
            if arr[0] == 0x66:
                seen_66 = True
            else:
                pp = {0xF2: "f2", 0xF3: "f3"}[arr[0]]
            arr = arr[1:]
        if pp == "none" and seen_66:
            pp = "66"
    else:
        # VEX/EVEX rows spell the instruction as `[mandatory prefix] opcode` in
        # one field: `shrx` is `0xf2f7` (VEX.F2.0F38.W0 F7), `shlxl` `0x66f7`.
        # The *last* byte is always the opcode; a leading byte in the prefix set
        # is the mandatory prefix.
        #
        # A single byte therefore has no mandatory prefix at all — the BMI rows
        # are exactly this shape, not a prefix with a missing opcode: `andn
        # 0xf2` is VEX.NDS.LZ.0F38.W0 F2, `blsi 0xf3/3` is VEX.NDS.LZ.0F38.W0
        # F3 /3, `vfpclassph 0x66` is EVEX.66.0F3A.W0 66.  The old loop treated
        # that lone byte as a prefix, left no opcode behind, and `len(arr) != 1`
        # below then dropped the row, which is why GNU as "had no entry" for
        # `andn`/`blsi`/`blsr`/`blsmsk` and the gate reported them as uncovered
        # instead of checked.
        while len(arr) > 1 and arr[0] in LEGACY_PREFIXES:
            pp = PP_FOR_BYTE[arr[0]]
            arr = arr[1:]
        if len(arr) == 1 and arr[0] in LEGACY_PREFIXES and "EVexMap" in attrs:
            # The APX `EVexMap4..7` promoted-legacy rows carry *only* a prefix
            # in the opcode field and take the opcode from the legacy row of the
            # same name (`wrssd, 0x66, ...|EVexMap4`).  There is nothing to
            # compare, and the maps themselves have no notation in this
            # snapshot: APX is modelled exactly by the SDM/XED gate.
            return None
    if map_ is None:
        if arr and arr[0] == 0x0F:
            arr = arr[1:]
            map_ = "0f"
            if arr and arr[0] in (0x38, 0x3A):
                map_ = "0f" + hex(arr[0])[2:]
                arr = arr[1:]
        else:
            map_ = "none"
    if map_ == "0f" and len(arr) == 2 and arr[0] == 0x01:
        # Three-byte-opcode rows: the field is `0F 01 <ModRM>`, where the ModRM
        # byte is the fixed form the row encodes and its `/reg` field is the
        # row's digit (`wrmsrns 0x0f01c6` is `0F 01 /0` with the fixed ModRM
        # `C6`).  When the row spells the digit itself (`sgdt 0x0f010/0`) that
        # spelling wins.
        if ext is None:
            ext = (arr[1] >> 3) & 7
        arr = arr[:1]
    if len(arr) != 1:
        return None
    fams = ["vex", "evex"] if family == "vexevex" else [family]
    imm_ext = "ImmExt" in attrs
    enc = {
        "family": family,
        "map": map_,
        "pp": pp,
        "w": w,
        "opcode": arr[0],
        "reg": None if imm_ext else ext,
        "imm_ext": ext if imm_ext else None,
        "vector": 256 if "Vex256" in attrs else (128 if "Vex128" in attrs else None),
    }
    return enc, fams


def _directions(enc: dict, attrs: str) -> list[dict]:
    """Expand the `D` attribute into the two opcode directions GAS really emits.

    `D` marks an instruction whose register/memory operands may appear in either
    order: `movups %xmm0,(%rax)` is `0f 11 00`, one byte above the `0f 10` load
    form in the table.  Without this expansion every store form in the encoder
    looks like an illegal encoding, which is exactly the false positive that
    makes a cross-reference tool untrustworthy.
    """
    base = {k: v for k, v in enc.items() if k != "directions"}
    tokens = set(attrs.split("|"))
    opcode, map_ = base["opcode"], base["map"]
    deltas: list[int] = []
    if "D" in tokens:
        if map_ == "none" and 0xD8 <= opcode <= 0xDF:
            # x87: the operand-order pair is the neighbouring arithmetic block
            # (D8 <-> DC, D9 <-> DD, DA <-> DE, DB <-> DF).
            deltas.append(0x04)
        else:
            deltas.append(0x01)
            if opcode in SSE_INT_MOVE:
                # MOVD/MOVQ/MOVDQA/MOVDQU: the store form is 0x10 above the load.
                deltas.append(0x10)
    if "W" in tokens:            # byte operand: base opcode; wider: base|1
        deltas.append(0x01)
    if "FloatMF" in tokens:      # x87 float memory form: dword as listed, qword ^4
        deltas.append(0x04)
    if not deltas:
        return [base]
    out = [base]
    for delta in deltas:
        paired = dict(base, opcode=base["opcode"] ^ delta)
        if paired["opcode"] not in {e["opcode"] for e in out}:
            out.append(paired)
    return out
def build_snapshot(tbl: Path) -> int:
    raw = tbl.read_bytes()
    text = raw.decode("utf-8", errors="replace")
    rows, templates, stats = parse_table(text)
    by_mnemonic: dict[str, list[dict]] = {}
    for row in rows:
        entry = {k: v for k, v in row.items() if k != "mnemonic"}
        by_mnemonic.setdefault(row["mnemonic"], []).append(entry)
        # GAS's front end prepends `v` to the vector-encoded rows, and the table
        # does not spell the letter out; BMI2 (`shrx`, `bzhi`, ...) and SM4 rows
        # are VEX-encoded but keep their plain name.  Registering both spellings
        # is what keeps this exact for either convention.
        if row["family"] != "legacy" and not row["mnemonic"].startswith("v"):
            by_mnemonic.setdefault("v" + row["mnemonic"], []).append(entry)
    for name, encodings in by_mnemonic.items():
        seen, unique = set(), []
        for enc in encodings:
            key = json.dumps(enc, sort_keys=True)
            if key not in seen:
                seen.add(key)
                unique.append(enc)
        by_mnemonic[name] = unique
    for encodings in by_mnemonic.values():
        encodings.sort(key=lambda e: (e["family"], e["map"], e["pp"], e["opcode"],
                                       -1 if e["reg"] is None else e["reg"]))
    SNAPSHOT.parent.mkdir(parents=True, exist_ok=True)
    SNAPSHOT.write_text(json.dumps(by_mnemonic, indent=0, sort_keys=True) + "\n")
    MANIFEST.write_text(json.dumps({
        "note": (
            "Encodings declared by GNU as's own instruction table, expanded from "
            "its template blocks.  Factual encoding data used as a test oracle; "
            "regenerate with scripts/gas_encoding_crossref.py --build <path to "
            "opcodes/i386-opc.tbl>.  Do not hand-edit."
        ),
        "upstream": {
            "repo": UPSTREAM_REPO,
            "path": UPSTREAM_PATH,
            "ref": UPSTREAM_REF,
            "implementation": "GNU as 2.47",
            "sha256": hashlib.sha256(raw).hexdigest(),
            "bytes": len(raw),
            "license": "GPL-3.0-or-later (source file); only derived encoding "
                       "facts are stored here",
        },
        "templates": {name: len(inst) for name, inst in sorted(templates.items())},
        "stats": stats,
        "mnemonics": len(by_mnemonic),
        "encodings": sum(len(v) for v in by_mnemonic.values()),
    }, indent=2, sort_keys=True) + "\n")
    print(f"{len(by_mnemonic)} mnemonics, {sum(len(v) for v in by_mnemonic.values())} "
          f"encodings, {stats['declarations']} templates expanded")
    return 0


def load_snapshot() -> dict[str, list[dict]]:
    return json.loads(SNAPSHOT.read_text())


# --------------------------------------------------------------------------
# Comparison against the encoder.
# --------------------------------------------------------------------------
SIZE_SUFFIX = re.compile(r"^(.*?)(b|w|l|q|s|t)$")

# Tokens the encoder uses to *compose* a mnemonic (the `cmp<ps|pd>`, `push %fs`
# tables) rather than as mnemonics of their own.  They are not rows to verify.
MNEMONIC_FRAGMENTS = {"ps", "pd", "ss", "sd", "fs", "gs", "es", "cs", "ds", "b", "w", "l", "q"}


def gas_name(mnemonic: str) -> str:
    """AT&T spelled the operand size as a suffix; the table does not.

    `addl` and `add` are one row in `i386-opc.tbl`, distinguished by operand
    type, so the encoder's suffixed spelling has to be folded back.  Some real
    mnemonics end in those letters (`call`, `mul`, `pop`), so the exact spelling
    is tried first by the caller.
    """
    m = SIZE_SUFFIX.match(mnemonic)
    return m.group(1) if m else mnemonic


# Encoder spellings that are aliases of a differently named table row.  The
# alias is resolved *before* the mnemonic is looked up, so the comparison is
# still exact: `setcc` is the encoder's generic spelling and it emits the row
# `setc` encodes (`0F 92`, SETC), which the snapshot holds under that name.
SNAPSHOT_ALIASES = {"setcc": "setc"}


# `i386-opc.tbl` spells a VEX and an EVEX instruction with the same template
# fields; which prefix family a row uses is not visible in the snapshot, so the
# three families compare as one when map, prefix and opcode agree.
VEX_PAIR = {frozenset(("vex", "legacy")), frozenset(("evex", "legacy")),
            frozenset(("vex", "evex"))}
# Mnemonics whose `i386-opc.tbl` entry cannot be compared with the encoder's row,
# with the reason.  Every one of them is pinned byte-for-byte by the as-2.47
# differential instead (`tests/asm-diff/prefix.casefile`), which compares
# against the assembler's actual output rather than against a transcription of
# its table.  `wrmsrns` used to be listed here; it is now distilled (0F 01 /0)
# and checked like every other row.
SNAPSHOT_LIMITATIONS = {
    # `movsb` is two different instructions in GAS: the string move (0xA4) and,
    # with operands, the sign-extending move (0F BE).  The snapshot holds only
    # the second spelling; the encoder row is the string form.
    "movsb": "string move vs sign-extend overload",
    "movsw": "string move vs sign-extend overload",
    "movsl": "string move vs sign-extend overload",
    "movsq": "string move vs sign-extend overload",
}

# Maps that exist only in EVEX/VEX and have no representation in i386-opc.tbl.
EXTENDED_MAPS = frozenset({"map4", "map5", "map6", "map7"})

HELPER_OF = re.compile(r"(?:self\.)?([a-z_][a-z0-9_]*)\(")


def _same(want: dict, have: dict) -> bool:
    if want.get("family") != have.get("family"):
        # A VEX/EVEX row and its legacy spelling share map, prefix and opcode;
        # which of the two families a row belongs to is a property of the
        # template attribute the row references, and `i386-opc.tbl` does not
        # always reference it (the `sqrt*` and `*dup` rows do not).  Comparing
        # them as one form is exact for the encoding and avoids a false report.
        if frozenset((want.get("family"), have.get("family"))) not in VEX_PAIR:
            return False
    # `i386-opc.tbl` has no notation for EVEX/VEX maps 4..7 (APX, USER_MSR), so
    # the snapshot records those forms with map "none".  The map bits are not
    # part of the comparison then; the SDM gate checks them exactly.
    want_map = want.get("map")
    if want_map in EXTENDED_MAPS:
        want_map = "none"
    for key, wv, hv in (("map", want_map, have.get("map")),
                        ("pp", want.get("pp"), have.get("pp")),
                        ("opcode", want.get("opcode"), have.get("opcode"))):
        if wv != hv:
            return False
    if want.get("reg") is not None and have.get("reg") is not None:
        return want["reg"] == have["reg"]
    if want.get("w") is not None and have.get("w") is not None:
        return want["w"] == have["w"]
    return True


def verify(rows: list[dict], table: dict[str, list[dict]],
           model) -> tuple[list, list, int, list]:
    """Check every modelled row against the snapshot.

    Returns `(findings, unmodelled, checked, uncovered)`: `uncovered` names the
    mnemonics the snapshot has no entry for at all (see the comment at the
    `known is None` branch)."""
    findings, unmodeLLed, uncovered, checked = [], [], [], 0
    for row in rows:
        # A row that binds a relocation kind or an operand name (`es`, `fs`,
        # `gotpcrel`, ...) is not an instruction row; the SDM gate skips exactly
        # these, so this one does too instead of counting them as unmodelled.
        if row["names"] and all(n in MNEMONIC_FRAGMENTS for n in row["names"]):
            continue
        if model(row) is None:
            unmodeLLed.append(row)
            continue
        for name in row["names"]:
            if name in MNEMONIC_FRAGMENTS or name in SNAPSHOT_LIMITATIONS:
                continue
            # The model depends on the spelling, not just on the row: one row
            # binds `lzcntl,tzcntl,popcntl` to the same helper and the opcode
            # comes from the mnemonic.
            encs = model(row, name)
            if encs is None:
                unmodeLLed.append(row)
                continue
            prefixes = [name]
            base = SNAPSHOT_ALIASES.get(name, gas_name(name))
            if base != name:
                prefixes.append(base)
            if name.startswith("v"):
                # Symmetric to the `v` synthesis: an AVX mnemonic whose row is
                # spelled without the letter is still that row.
                prefixes.append(name[1:])
            known = next((table[p] for p in prefixes if p in table), None)
            if known is None:
                # No entry under any spelling of the name.  This list is a
                # ratchet and is empty today: the BMI rows (`andn`, `blsi`,
                # `blsr`, `blsmsk`) used to be dropped by the distiller's
                # prefix handling, and `setcc` is an alias resolved through
                # `SNAPSHOT_ALIASES`.  Anything that reappears here is a
                # reference gap to close, not noise — `--strict` reports it.
                uncovered.append({"name": name, "file": row["file"],
                                  "line": row["line"]})
                continue
            checked += 1
            # The row model is operand-shape-agnostic: `encode_mov` covers
            # register, memory and immediate forms in one row, and the snapshot
            # was distilled from the operand forms `as` was probed with.  The
            # check is therefore "the row can emit something `as` also emits",
            # reported per unmatched encoding only when nothing matches at all;
            # the SDM gate is the one that requires *every* modelled form to be
            # a legal encoding of the mnemonic.
            extra = [enc for enc in encs if not any(_same(enc, have) for have in known)]
            if len(extra) == len(encs):
                findings.append({"kind": "LCCC-EXTRA", "name": name,
                                 "file": row["file"], "line": row["line"],
                                 "body": row["body"], "want": extra[0]})
    return findings, unmodeLLed, checked, uncovered


def missing(rows: list[dict], table: dict[str, list[dict]], model) -> dict[str, list[dict]]:
    """Encodings `as` accepts for mnemonics the encoder already knows."""
    have: dict[str, list[dict]] = {}
    for row in rows:
        encs = model(row)
        if encs is None:
            continue
        for name in row["names"]:
            base = gas_name(name)
            have.setdefault(name, []).extend(encs)
            have.setdefault(base, []).extend(encs)
    gaps: dict[str, list[dict]] = {}
    for name, encs in table.items():
        if name not in have:
            continue
        mine = have[name]
        for enc in encs:
            if not any(_same(enc, m) for m in mine):
                gaps.setdefault(name, []).append(enc)
    return gaps


def unimplemented_mnemonics(rows: list[dict], table: dict[str, list[dict]]) -> list[str]:
    named = set()
    for row in rows:
        for name in row["names"]:
            named.add(name)
            named.add(gas_name(name))
    return sorted(n for n in table if n not in named)


def coverage(rows: list[dict], table: dict[str, list[dict]]) -> dict[str, list[dict]]:
    """Reference encodings for every mnemonic the encoder has no row for."""
    have = set()
    for row in rows:
        for name in row["names"]:
            have.add(name)
            have.add(gas_name(name))
    gap: dict[str, list[dict]] = {}
    for name in sorted(table):
        if name in have:
            continue
        forms = [e for e in table[name] if e["family"] != "evex"]
        if forms:
            gap[name] = forms
    return gap


def triage_absent(absent: list[str], files: list[str]) -> tuple[list[str], list[str]]:
    """Split `absent` into (unmentioned, mentioned) over the backend sources.

    Both halves need care, which is why this is a reading aid and not a gate:

    * MENTIONED may be a comment, a deny-list entry, or a family prefix
      (`cmov`, `set`) whose arm does not cover the spelling.
    * UNMENTIONED may still be supported: an encoder that composes the mnemonic
      (`format!("vfmadd{op}...")`, `concat!`) has no literal for it.

    What it does buy is the *order* to read the list in: a name that appears
    nowhere at all is the cheapest place to start.  One scan of the whole source
    per name is why this runs interactively, never in CI.
    """
    text = "\n".join(Path(f).read_text(encoding="utf-8", errors="replace") for f in files)
    unmentioned, mentioned = [], []
    for name in absent:
        pat = re.compile(r"(?<![A-Za-z0-9_.])" + re.escape(name) + r"(?![A-Za-z0-9_])")
        (mentioned if pat.search(text) else unmentioned).append(name)
    return unmentioned, mentioned


def _fmt(enc: dict) -> str:
    parts = [enc["family"], enc.get("map") or "-", enc.get("pp") or "-"]
    if enc.get("w") is not None:
        parts.append(f"W{enc['w']}")
    parts.append(f"{enc['opcode']:02x}")
    if enc.get("reg") is not None:
        parts.append(f"/{enc['reg']}")
    return " ".join(parts)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--backend", default="i686", choices=["all", "i686", "x86"],
                    help="encoder backend(s) to check (default: i686)")
    ap.add_argument("--build", metavar="TBL",
                    help="regenerate the snapshot from an i386-opc.tbl")
    ap.add_argument("--verify", action="store_true", help="check every encoder row")
    ap.add_argument("--missing", action="store_true", help="encodings the encoder lacks")
    ap.add_argument("--absent", action="store_true", help="mnemonics with no row at all")
    ap.add_argument("--triage", action="store_true",
                    help="with --absent: split by whether the backend sources mention the name")
    ap.add_argument("--coverage", metavar="OUT",
                    help="write a JSON worklist of mnemonics with no encoder row")
    ap.add_argument("--summary", action="store_true")
    ap.add_argument("--strict", action="store_true", help="non-zero exit on findings")
    ap.add_argument("--limit", type=int, default=0)
    args = ap.parse_args()

    if args.build:
        return build_snapshot(Path(args.build))

    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import sdm_isa_crossref as sdm  # local sibling module

    # Rows that bind a relocation kind or an operand name are not instructions;
    # the SDM gate skips exactly the same set, so this gate does too.
    MNEMONIC_FRAGMENTS.update(sdm.NOT_A_MNEMONIC)

    table = load_snapshot()
    rows = []
    files = sdm.encoder_files(args.backend)
    for path in files:
        rows.extend(sdm.extract_rows(Path(path).resolve()))
    # REX (`40..4F`) is a prefix in the 64-bit encoder and an inc/dec opcode in
    # the 32-bit one, so the model has to know which backend it is reading.
    # The mode that gives a byte array its meaning is a property of the row's
    # backend, not of the whole run: `--backend all` mixes both.
    # The model is a function of the row *and* the mnemonic: several helpers
    # resolve their opcode from the spelling (`lzcnt` vs `popcnt`, `setc` vs
    # `sete`), so a row is modelled per name exactly as the SDM gate does it.
    def _name_of(row):
        return row["names"][0] if row["names"] else None

    def model(row, name=None):
        """Model one row as the encoding of `name` (default: its first name)."""
        name = name if name is not None else _name_of(row)
        if args.backend == "all":
            return sdm.model_any(row, name)
        return sdm.model_row(row, args.backend == "x86", name)

    if args.summary or not (args.verify or args.missing or args.absent):
        manifest = json.loads(MANIFEST.read_text())
        print(f"as 2.47 table: {manifest['mnemonics']} mnemonics, "
              f"{manifest['encodings']} encodings, "
              f"{manifest['stats']['declarations']} templates, "
              f"{manifest['stats']['instantiations']} instantiation rows, "
              f"{manifest['stats']['skipped']} skipped")
        print(f"encoder rows:  {len(rows)} in {len(files)} files")

    if args.verify:
        findings, unmodeLLed, checked, uncovered = verify(rows, table, model)
        print(f"\nrows checked against as 2.47: {checked}")
        print(f"findings: {len(findings)}   unmodeLLed rows: {len(unmodeLLed)}"
              f"   not in the as 2.47 snapshot: {len(uncovered)}")
        shown = 0
        for f in findings:
            if args.limit and shown >= args.limit:
                print(f"  ... {len(findings) - shown} more")
                break
            shown += 1
            want = f" [{_fmt(f['want'])}]" if f["want"] else ""
            print(f"  {f['kind']:13s} {f['name']:14s} {f['file']}:{f['line']}{want}")
            print(f"                {f['body'][:110]}")
        # A row the model cannot read is only actionable when it is a binding
        # the model *should* understand; the hundreds that come from prefix and
        # tuple tables are pure noise, so they are counted by helper instead of
        # listed one by one.  `--limit` still prints individual rows on demand.
        if unmodeLLed:
            groups: dict[str, int] = {}
            for row in unmodeLLed:
                helper = HELPER_OF.search(row["body"])
                key = helper.group(1) if helper else row["body"].split("(")[0].strip()
                groups[key] = groups.get(key, 0) + 1
            for key in sorted(groups, key=lambda k: (-groups[k], k)):
                print(f"  UNMODELLED    {groups[key]:5d} rows via {key}")
            for row in unmodeLLed[: args.limit]:
                print(f"  UNMODELLED    {row['names'][0]:14s} {row['file']}:{row['line']}")
                print(f"                {row['body'][:110]}")
        if uncovered:
            names = sorted({u["name"] for u in uncovered})
            print(f"  UNCOVERED     {len(names)} mnemonic(s) with no entry in the "
                  f"as 2.47 snapshot: {' '.join(names)}")
        if args.strict and (findings or unmodeLLed):
            return 1

    if args.missing:
        gaps = missing(rows, table, model)
        total = sum(len(v) for v in gaps.values())
        print(f"\nencodings as 2.47 accepts that no declarative row models: {total} "
              f"across {len(gaps)} mnemonics with at least one row "
              f"(code-implemented families are not rows; see the docstring)")
        for name in sorted(gaps):
            forms = ", ".join(_fmt(e) for e in gaps[name])
            print(f"  {name:14s} {forms}")

    if args.coverage:
        gap = coverage(rows, table)
        Path(args.coverage).write_text(json.dumps(gap, indent=1, sort_keys=True) + "\n")
        legal = sum(len(v) for v in gap.values())
        print(f"\nmnemonics with no declarative row (written to {args.coverage}):"
              f" {len(gap)} carrying {legal} reference encodings")

    if args.absent:
        absent = unimplemented_mnemonics(rows, table)
        print(f"\nmnemonics as 2.47 knows with no declarative row: {len(absent)}")
        if args.triage:
            unmentioned, mentioned = triage_absent(absent, files)
            print(f"  UNMENTIONED {len(unmentioned)}: the spelling appears in no "
                  f"backend source -- read these first, not a verdict")
            print("    " + " ".join(unmentioned))
            print(f"  MENTIONED   {len(mentioned)}: named somewhere in the sources; "
                  f"candidates, not proof (a comment or a deny list names them too)")
            return 0
        print("  " + " ".join(absent))
    return 0


if __name__ == "__main__":
    sys.exit(main())
