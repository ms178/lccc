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
                 or an invented mnemonic (LCCC-UNKNOWN); plus every encoding a
                 modelled *code* path derives (`CODE-EXTRA`/`CODE-UNKNOWN`).
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
helpers, so none of them has a row.  The gap counts therefore blame the model
rather than the encoder: with ``--backend all`` the tree reported `add 01` under
``--missing`` before `encode_alu` was modelled, and still reports alias
spellings such as `cmovle`, `jnae` and `jrcxz` under ``--absent`` although a
suffix arm encodes them, while `add`, `call`, `movbe`, `rorx`, `pdep`, `pext`,
`mulx` and `andn` are *not* listed there because some row names them.  The two
counts are a *declarative-row* census, not an ISA-coverage census, and the sound
direction is ``--verify`` — it asks whether each row that does exist is a legal
encoding.

The cure is to model code only where the backend demonstrably emits it.
``code_models()`` covers the integer ALU, condition-code spellings, i686
one-byte `inc`/`dec`, APX EVEX map-4 ALU/shift/double-shift/unary/IMUL/bit-count/inc-dec and
condition-code forms, and the FMA3 grid. APX opcode formulas are derived from
the encoder dispatch/helper implementations; all claims are checked against
the pinned GAS 2.47 snapshot. ``--missing`` must pass the same snapshot into
``code_models()`` so snapshot-derived models (including FMA3 spellings) are
included consistently in the report. A model is not proof that the encoder is
correct: it proves only that the claimed form exists in GAS's opcode table.
The remaining families are the worklist in `backlog.md` MB-45, and every
reported gap still needs source-path classification and byte-level regression
coverage before it can be called an implementation omission.

The snapshot's own transcription is the other half of the story, and it is
checkable: `D` is not "XOR 1" (GNU as uses four different schemes and leaves
three cases unflipped), `D` and `W` compose, and rows without a ModR/M keep
their register in the opcode (`push %rcx` is `51`).  Getting those wrong makes
the oracle accept `movdqa 66 0f 6e` (which is `movd`) and reject `and
(%rax),%al` (`22 00`) — the two failure modes a cross-reference tool must not
have.  ``tests/corpus/test_gas_snapshot_decode.py`` anchors the transcription to
``objdump`` instead of to a second reading of the table.

`--backend {all,i686,x86}` selects the encoder sources.  The 64-bit backend is
not a superset of the 32-bit one — x87 integer memory arithmetic, `fcmov*` and
`fucom*` live only in `i686` — so use `--backend all` before a release (the CLI default remains `i686`).

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
import signal
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
    """Evaluate an opcode expression with C precedence: `|` < `<<` < `+` < `*`."""
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
        val = add()
        while peek() == "<<":
            pos += 1
            val <<= add()
        return val

    def mul() -> int:
        nonlocal pos
        val = primary()
        while peek() == "*":
            pos += 1
            val *= primary()
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
        val = shift()
        while peek() == "|":
            pos += 1
            val |= shift()
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


def _parse_row(line: str) -> list[dict] | None:
    parts = _split_top(line)
    if len(parts) < 4:
        return None
    mnemonic = parts[0].strip()
    if not mnemonic or not re.fullmatch(r"[a-z][a-z0-9]*", mnemonic):
        return None
    opcode_field = parts[1].strip()
    cpu = parts[2].strip()
    attrs = parts[3].strip()
    operands = parts[4].strip() if len(parts) > 4 else ""
    # i386-gen prefixes only SSE2AVX template names; VEX itself does not
    # invent a v spelling (vandn/vblsi/vshrx are not GAS mnemonics).
    if "SSE2AVX" in {t.strip() for t in attrs.split("|")}:
        mnemonic = "v" + mnemonic
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
        one["directions"] = _directions(one, attrs, cpu, operands)
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
    tokens = {t.strip() for t in attrs.split("|")}
    # VexW0/VexW1 describe W for either prefix family; they do not enable
    # VEX. Likewise "Vex" is a substring of "EVex", not a second family.
    vex = bool(tokens & {"Vex", "Vex128", "Vex256", "VexLIG"})
    evex = bool(tokens & {"EVex", "EVex128", "EVex256", "EVex512",
                          "EVexLIG", "EVexDYN", "EVexMap4", "EVexMap5",
                          "EVexMap6", "EVexMap7"})
    vex = vex or any(t.startswith("Vex=") for t in tokens)
    evex = evex or any(t.startswith("EVex=") for t in tokens)
    # GAS also uses implicit EVEX for masked AVX512 rows without an EVexLIG
    # attribute (e.g. vprold). Masking is an exact attribute, not a substring.
    evex = evex or "Masking" in tokens
    family = "vexevex" if vex and evex else "evex" if evex else "vex" if vex else "legacy"
    w = None
    # i386-opc.h defines VEXW0=1, VEXW1=2, VEXWIG=3.
    if tokens & {"VexW1", "VexW=2", "VexW=VEXW1"}:
        w = 1
    elif tokens & {"VexW0", "VexW=1", "VexW=VEXW0"}:
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
        # F3 /3, `vfpclassph 0x66` is EVEX.NONE.0F3A.W0 66.  The old loop treated
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


def _direction_mask(enc: dict, attrs: str, cpu: str) -> int:
    """The opcode bit `D` flips, exactly as `tc-i386.c` chooses it.

    `D` does not mean "flip one bit": `tc-i386.c` picks one of four schemes,
    and three of the cases change nothing at all.  Transcribed from
    `install_template` / `match_template`:

      * `Ugh` rows (`faddp`, `fsubrp`, ...) are *rewritten* rather than
        re-encoded — `faddp %st(1),%st` assembles as `faddp %st,%st(1)`, both
        `de c1`, so no opcode moves (`found_reverse_match = ~0`);
      * FMA4 and XOP put the operand order in VEX.W (`Opcode_VexW`), so again
        no opcode moves — the scalar FMA4 block is `6a/6b/6e/6f/7a/7b/7e/7f`
        for eight *different* instructions, and flipping "the direction bit"
        there invents a spelling that belongs to its neighbour;
      * x87 register forms are the tbyte case (`FloatReg Class=RegFP|Tbyte`):
        the reverse form is the neighbouring arithmetic block, `D8` <-> `DC`
        (`Opcode_FloatD`, 0x4);
      * base-map rows XOR `Opcode_D` (0x2), which includes the APX `EVexMap4`
        rows — they keep `SPACE_MAP4` but take the base decision at 3+ operands
        and for promoted two-operand forms;
      * the extended maps XOR `Opcode_ExtD` (0x1) — except the compact SIMD
        integer moves, whose `6e/6f` and `7e/7f` pairs are their own
        load/store partners (`Opcode_SIMD_IntD`, 0x10), and `mov` to/from
        CR/DR/TR (`0f 20..27`), which the table marks `NoRex64` and GAS keeps
        on the base-map scheme:

            i.tm.base_opcode ^= (base_opcode & 0xee) != 0x6e
                                ? Opcode_ExtD : Opcode_SIMD_IntD;

    Getting this wrong is not academic: the 0x1 scheme on `movdqa 0x66 0f 6f`
    claims `66 0f 6e`, which is `movd`, so the oracle accepted an invented
    encoding — the exact defect this file exists to prevent.
    """
    tokens = {t.strip() for t in attrs.split("|")}
    opcode, map_ = enc["opcode"], enc["map"]
    if "Ugh" in tokens or "FMA4" in cpu or "XOP" in cpu:
        return 0
    if tokens & {"C", "StaticRounding"}:
        # `C` is `#define C StaticRounding`: the bit GAS reads as
        # `opcode_modifier.commutative` for operand-order purposes.  A
        # commutative row keeps its opcode when the operands are reversed --
        # `test %eax,(%rax)` and `test (%rax),%eax` are both `85 00`, and
        # `xchg` likewise, so flipping a bit here would invent `86`/`87` for
        # `test` (which are `xchg`).
        return 0
    if map_ == "none" and 0xD8 <= opcode <= 0xDF:
        return 0x04
    if map_ == "none":
        return 0x02
    if map_ == "0f" and 0x20 <= opcode <= 0x27:
        return 0x02
    return 0x10 if (opcode & 0xEE) == 0x6E else 0x01


GPR_OPERAND = re.compile(r"^Reg(?:8|16|32|64)(?:\|Reg(?:8|16|32|64))*$")


def _register_opcode(attrs: str, operands: str) -> bool:
    """Does GAS put this row's register number in the opcode byte (`+rd`)?

    `push %rcx` is `51`, `bswap %ecx` is `0f c9`, `mov $1,%ecx` is `b9`: one
    row covers eight encodings because the ModR/M byte has nowhere to go.
    Without this the snapshot knew only `0x50` for `push` and would have called
    the encoder's `pushq %rcx` an invented encoding.

    The test is structural rather than a list of mnemonics: no ModR/M and no
    `RegMem` (those put the register in ModR/M), and exactly one operand that
    is a bare GPR group.  `Acc` is deliberately excluded — `in $1,%al` is `e4
    01`, whose accumulator is implied, and `push %fs` (`0f a0`) takes its
    register from a fixed segment-register row instead.
    """
    tokens = {t.strip() for t in attrs.split("|")}
    if tokens & {"Modrm", "RegMem"}:
        return False
    body = operands.strip()
    if not (body.startswith("{") and body.endswith("}")):
        return False
    terms = [t.strip() for t in _split_top(body[1:-1]) if t.strip()]
    return sum(1 for t in terms if GPR_OPERAND.match(t)) == 1


def _directions(enc: dict, attrs: str, cpu: str = "",
                operands: str = "") -> list[dict]:
    """Every opcode the assembler derives from one table row.

    Three modifiers move the opcode within a row, and because they flip
    distinct bits they compose rather than replace each other:

      * `D`   -- the operand-order bit (see `_direction_mask`);
      * `W`   -- byte operands use the listed opcode, wider ones `opcode | 1`
                 (`mov 0x88` is `mov %al,(%rax)`, `0x89` is `mov %eax,(%rax)`);
      * `FloatMF` -- x87's memory-operand size bit (`fld 0xd9/0` is the dword
                 form, `0xdd/0` the qword one).

    The cross product matters: `mov` carries both `D` and `W`, and `as`
    accepts all four of `88`, `89`, `8a`, `8b`.  Enumerating the modifiers
    one-dimensionally produced only three and silently declared the fourth
    illegal -- the same class of false positive this whole expansion exists to
    remove.

    Rows that carry a register in the opcode (`+rd`) additionally expand over
    the eight registers (see `_register_opcode`).
    """
    base = {k: v for k, v in enc.items() if k != "directions"}
    tokens = {t.strip() for t in attrs.split("|")}
    masks: list[int] = []
    if "D" in tokens:
        mask = _direction_mask(base, attrs, cpu)
        if mask:
            masks.append(mask)
    if "W" in tokens:
        masks.append(0x01)
    if "FloatMF" in tokens:
        masks.append(0x04)
    opcodes = [base["opcode"]]
    for mask in masks:
        opcodes.extend(op ^ mask for op in list(opcodes))
    if _register_opcode(attrs, operands):
        opcodes = [op | rd for op in opcodes for rd in range(8)]
    out: list[dict] = []
    seen: set[int] = set()
    for opcode in opcodes:
        if opcode in seen:
            continue
        seen.add(opcode)
        out.append(base if opcode == base["opcode"] else dict(base, opcode=opcode))
    return out


def build_snapshot(tbl: Path) -> int:
    raw = tbl.read_bytes()
    text = raw.decode("utf-8", errors="replace")
    rows, templates, stats = parse_table(text)
    by_mnemonic: dict[str, list[dict]] = {}
    for row in rows:
        entry = {k: v for k, v in row.items() if k != "mnemonic"}
        by_mnemonic.setdefault(row["mnemonic"], []).append(entry)
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
    # The bare spellings collide with the SSE scalar compares/moves after the
    # AT&T size suffix is folded: `movss` and `cmpss` reduce to `movs`/`cmps`,
    # which are the *string* instructions in the table (`A4/A5`, `A6/A7`).
    "movs": "string move vs `movss` after suffix folding",
    "cmps": "string compare vs `cmpss` after suffix folding",
}

# Maps that exist only in EVEX/VEX and have no representation in i386-opc.tbl.
EXTENDED_MAPS = frozenset({"map4", "map5", "map6", "map7"})

HELPER_OF = re.compile(r"(?:self\.)?([a-z_][a-z0-9_]*)\(")


def _same(want: dict, have: dict) -> bool:
    if want.get("family") != have.get("family"):
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
    for key in ("reg", "w"):
        if want.get(key) is not None and have.get(key) is not None:
            if want[key] != have[key]:
                return False
    return True


def verify(rows: list[dict], table: dict[str, list[dict]],
           model) -> tuple[list, list, int, list]:
    """Check every modelled row against the snapshot.

    Returns `(findings, unmodelled, checked, uncovered)`: `uncovered` names the
    mnemonics the snapshot has no entry for at all (see the comment at the
    `known is None` branch)."""
    findings, unmodelled, uncovered, checked = [], [], [], 0
    for row in rows:
        # A row that binds a relocation kind or an operand name (`es`, `fs`,
        # `gotpcrel`, ...) is not an instruction row; the SDM gate skips exactly
        # these, so this one does too instead of counting them as unmodelled.
        if row["names"] and all(n in MNEMONIC_FRAGMENTS for n in row["names"]):
            continue
        if model(row) is None:
            unmodelled.append(row)
            continue
        for name in row["names"]:
            if name in MNEMONIC_FRAGMENTS or name in SNAPSHOT_LIMITATIONS:
                continue
            # The model depends on the spelling, not just on the row: one row
            # binds `lzcntl,tzcntl,popcntl` to the same helper and the opcode
            # comes from the mnemonic.
            encs = model(row, name)
            if encs is None:
                unmodelled.append(row)
                continue
            prefixes = [name]
            base = SNAPSHOT_ALIASES.get(name, gas_name(name))
            if base != name:
                prefixes.append(base)
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
    return findings, unmodelled, checked, uncovered


# ---------------------------------------------------------------------------
# Code-driven families (MB-45).
#
# Not every instruction the backend encodes has a declarative row: `encode_alu`
# derives its opcodes arithmetically, `encode_jcc`/`encode_cmovcc`/
# `encode_setcc` resolve a condition-code alias through a table, and FMA3 reads
# its opcode out of the mnemonic's own digits.  The row model cannot see any of
# them, so the gap counts blamed the encoder for the model's blind spots.
#
# Each model below is a *claim* about what the encoder can emit, derived from
# the encoder's own source — the dispatch arms, the arithmetic in `encode_alu`,
# the alias table in `cc_from_mnemonic`, the grid in `fma3_opcode`.  Claims are
# not trusted: `verify_code_models` requires every claimed encoding to exist in
# the `as` 2.47 snapshot under that mnemonic, so a model that drifts from the
# encoder (or an encoder that drifts from `as`) fails the gate instead of
# quietly shrinking the gap list.  That check is the reason these families are
# worth modelling at all: before it, no gate looked at an ALU encoding.
# ---------------------------------------------------------------------------

# `"addq" | "addl" | "addw" | "addb" => self.encode_alu(ops, mnemonic, 0),`
ALU_ARM = re.compile(
    r'((?:"[a-z0-9]+"\s*\|\s*)*"[a-z0-9]+")\s*=>\s*'
    r'self\.encode_alu\(ops,\s*mnemonic,\s*(\d+)\)'
)
QUOTED = re.compile(r'"([a-z0-9]+)"')


def _enc(opcode: int, *, family: str = "legacy", map_: str = "none",
         pp: str = "none", w: int | None = None,
         reg: int | None = None) -> dict:
    """One encoding, in the exact shape the snapshot uses for comparisons."""
    return {"family": family, "map": map_, "pp": pp, "w": w,
            "opcode": opcode, "reg": reg}


def _dedupe(models: dict[str, list[dict]]) -> dict[str, list[dict]]:
    out: dict[str, list[dict]] = {}
    for name, encs in models.items():
        seen: dict[tuple, dict] = {}
        for enc in encs:
            seen[(enc["family"], enc["map"], enc["pp"], enc["opcode"],
                  enc["reg"], enc["w"])] = enc
        out[name] = list(seen.values())
    return out


def _fn_body(path: Path, name: str) -> str | None:
    """The brace-matched body of `fn name(...)`, or `None`.

    Brace matching rather than "find the next `\n}`": these functions contain
    nested blocks and string literals, and a truncated body silently drops
    table rows, which is the failure mode the models exist to avoid.
    """
    text = path.read_text(encoding="utf-8", errors="replace")
    head = text.find(f"fn {name}(")
    if head < 0:
        return None
    open_brace = text.find("{", head)
    if open_brace < 0:
        return None
    depth = 0
    for i in range(open_brace, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[open_brace + 1:i]
    return None


def alu_forms(op: int) -> list[dict]:
    """The nine legacy encodings `encode_alu` emits for dispatch index `op`.

    `op` is the ALU index the dispatch arm passes (0 = add, 4 = and, 7 = cmp).
    The six opcode forms carry it in the opcode byte (`0x01 + op * 8`), the
    three immediate forms carry it in ModR/M's `/digit` field with a fixed
    opcode — 80 imm8, 81 imm32, 83 sign-extended imm8.  Deriving both from `op`
    is what makes this a model of the code rather than a second table: `add`
    lands on 00..05 and `and` on 20..25 from the same two lines.
    """
    forms = [_enc(base + op * 8) for base in (0x00, 0x01, 0x02, 0x03, 0x04, 0x05)]
    forms.extend(_enc(opcode, reg=op) for opcode in (0x80, 0x81, 0x83))
    return forms


def alu_code_models() -> dict[str, list[dict]]:
    """`{gas mnemonic: [encoding, ...]}` for every `encode_alu` dispatch arm."""
    text = (REPO / "src/backend/x86/assembler/encoder/mod.rs").read_text(
        encoding="utf-8", errors="replace")
    out: dict[str, list[dict]] = {}
    for names, op in ALU_ARM.findall(text):
        forms = alu_forms(int(op))
        for name in QUOTED.findall(names):
            out.setdefault(gas_name(name), []).extend(forms)
    return _dedupe(out)


# `cc_from_mnemonic` lives in each backend's `registers.rs`; the two tables are
# byte-identical today, and modelling both means a divergence shows up here
# instead of as a silent gap in one backend.
CC_FILES = {
    "i686": "src/backend/i686/assembler/encoder/registers.rs",
    "x86": "src/backend/x86/assembler/encoder/registers.rs",
}
# `"b" | "c" | "nae" => Ok(2),`
CC_ARM = re.compile(r'((?:"[a-z]+"\s*\|\s*)*"[a-z]+")\s*=>\s*Ok\((\d+)\)')


def cc_code_models(backend: str = "all") -> dict[str, list[dict]]:
    """`j<cc>`, `cmov<cc>` and `set<cc>` for every alias `cc_from_mnemonic` takes.

    Only the SHORT `jcc` form is claimed: `as`'s table carries `0x70+cc` under
    `j<cc>` and leaves the near relaxation (`0F 8x`) to its front end, so a
    claim on `0F 8x` would be a claim the oracle cannot confirm — the model
    fails closed rather than reporting a phantom disagreement.
    """
    out: dict[str, list[dict]] = {}
    backends = ("i686", "x86") if backend == "all" else (backend,)
    for be in backends:
        body = _fn_body(REPO / CC_FILES[be], "cc_from_mnemonic")
        if body is None:
            continue
        for names, code in CC_ARM.findall(body):
            cc = int(code)
            for alias in QUOTED.findall(names):
                out.setdefault(f"j{alias}", []).append(_enc(0x70 + cc))
                out.setdefault(f"cmov{alias}", []).append(_enc(0x40 + cc, map_="0f"))
                out.setdefault(f"set{alias}", []).append(
                    _enc(0x90 + cc, map_="0f", reg=0))
                if be == "x86":
                    out[f"cmov{alias}"].append(_enc(0x40 + cc, family="evex"))
                    # APX map-4 promotes byte register destinations to the
                    # zero-extending SETcc form; GAS drops legacy F2 and uses
                    # opcode 40+cc, ModR/M /0. The x86 APX path emits it.
                    out[f"set{alias}"].append(
                        _enc(0x40 + cc, family="evex", pp="f2", reg=0))
    return _dedupe(out)


# `incq`/`decq` in the i686 backend use the one-byte `40+rd`/`48+rd` forms for a
# 32-bit register; the 64-bit backend cannot (REX owns `40..4F`) and goes
# through `FE`/`FF`, which are declarative rows there.
INCDEC_I686 = {"inc": 0x40, "dec": 0x48}


def incdec_code_models(backend: str = "all") -> dict[str, list[dict]]:
    if backend not in ("i686", "all"):
        return {}
    return {name: [_enc(opcode)] for name, opcode in INCDEC_I686.items()}


FMA_ELEM = {"ps": (0, 0), "pd": (1, 0), "ss": (0, 1), "sd": (1, 1)}
FMA_ORDER = {"132": 0, "213": 1, "231": 2}
FMA_BASE = {("add", False): 0x98, ("sub", False): 0x9A,
            ("add", True): 0x9C, ("sub", True): 0x9E}
FMA_PACKED_ONLY = {"addsub": 0x96, "subadd": 0x97}


def fma3_encoding(mnemonic: str) -> dict | None:
    """Mirror of `fma3_opcode` in `encoder/registers.rs` (identical in i686).

    The encoder reads the opcode out of the mnemonic itself —
    `vfm<op><order><elem>` — so no row can carry it.  Reimplementing the grid
    is the only way to model it, and `verify_code_models` is what makes the
    reimplementation trustworthy: every derived opcode must match `as` 2.47.
    """
    if mnemonic.startswith("vfm"):
        rest, negated = mnemonic[3:], False
    elif mnemonic.startswith("vfnm"):
        rest, negated = mnemonic[4:], True
    else:
        return None
    if len(rest) < 5:
        return None
    elem, body = rest[-2:], rest[:-2]
    if elem not in FMA_ELEM:
        return None
    w, scalar = FMA_ELEM[elem]
    op, order = body[:-3], body[-3:]
    if order not in FMA_ORDER:
        return None
    base = FMA_BASE.get((op, negated))
    if base is None:
        # addsub/subadd have no scalar form and no negated spelling.
        if scalar or negated:
            return None
        base = FMA_PACKED_ONLY.get(op)
        if base is None:
            return None
    return _enc(base + 0x10 * FMA_ORDER[order] + scalar,
                family="vex", map_="0f38", pp="66", w=w)


def fma3_code_models(table: dict[str, list[dict]]) -> dict[str, list[dict]]:
    """FMA3 encodings for every spelling the table knows and the grid accepts.

    The name set comes from the reference table because the encoder's dispatch
    is `m if is_fma3_vex(m)` — it accepts every mnemonic the grid parses — and
    the table is the only list of the spellings `as` will assemble.  The
    *encodings* are derived from the grid, which is what gets verified.
    """
    out: dict[str, list[dict]] = {}
    for name in table:
        enc = fma3_encoding(name)
        if enc is not None:
            out[name] = [enc]
    return out


def _apx_alu_models() -> dict[str, list[dict]]:
    """APX EVEX map-4 forms emitted by `encode_alu_apx`.

    `encode_alu_apx` is the implementation used for explicit `{evex}` / `{nf}`
    2-operand forms and 3-operand NDD forms. The APX map keeps the legacy
    opcode byte: its four r/m<->reg directions are `00..03 + op*8`; immediate
    groups are `80 /op`, `81 /op`, `83 /op`. It intentionally has no accumulator
    short encodings. `cmp` uses APX NCI/CCMP instead and is not this helper.
    These formulas are derived from `alu_forms` and the Rust dispatch indices;
    `verify_code_models` checks each emitted claim against GAS 2.47.
    """
    out = {}
    for name, forms in alu_code_models().items():
        if name not in ALU_DIGITS:
            continue
        forms = [e for e in forms
                 if e["opcode"] < 0x80 or e["opcode"] in (0x80, 0x81, 0x83)]
        # `04+op*8`/`05+op*8` are the accumulator-immediate shortcuts and do
        # not survive the APX NDD form; keep only the four ModR/M directions.
        op = ALU_DIGITS[name]
        forms = [e for e in forms if e["opcode"] not in (0x04 + op * 8,
                                                           0x05 + op * 8)]
        out[name] = [dict(e, family="evex", map="none") for e in forms]
    return out


ALU_DIGITS = {"add": 0, "or": 1, "adc": 2, "sbb": 3,
              "and": 4, "sub": 5, "xor": 6}
APX_SHIFT_ARM = re.compile(
    r'((?:"[a-z0-9]+"\s*\|\s*)*"[a-z0-9]+")\s*=>\s*'
    r'(?:\{\s*)?self\.encode_shift\(ops,\s*mnemonic,\s*(\d+)\)'
)
APX_INCDEC_ARM = re.compile(
    r'"(inc|dec)[a-z]?"\s*=>\s*self\.encode_inc_dec\(ops,\s*([01]),\s*([1248])\)'
)


def apx_integer_code_models() -> dict[str, list[dict]]:
    """Map-4 opcodes for x86 integer helper paths that explicitly emit APX.

    This deliberately covers only helpers whose implementation has an APX
    branch (`encode_alu`, `encode_shift`, `encode_inc_dec`, `encode_imul`,
    `encode_bit_count`, and the cc helpers). BMI, MOVBE/MOVDIR and CRC32 get
    separate evidence/model work rather than a blanket EVEX-family guess.
    """
    out = _apx_alu_models()
    text = (REPO / "src/backend/x86/assembler/encoder/mod.rs").read_text(
        encoding="utf-8", errors="replace")
    for names, digit_s in APX_SHIFT_ARM.findall(text):
        digit = int(digit_s)
        # `encode_shift_apx` puts the group digit in ModR/M.reg for all six
        # count forms; the opcode chooses imm8, 1, or CL and byte vs wider size.
        forms = [_enc(opcode, family="evex", reg=digit)
                 for opcode in (0xC0, 0xC1, 0xD0, 0xD1, 0xD2, 0xD3)]
        for name in QUOTED.findall(names):
            out.setdefault(gas_name(name), []).extend(forms)
    for name, digit_s, size_s in APX_INCDEC_ARM.findall(text):
        digit = int(digit_s)
        out.setdefault(name, []).extend(
            _enc(opcode, family="evex", reg=digit) for opcode in (0xFE, 0xFF))
    # `encode_imul` explicitly uses APX EVEX for unary, two-operand and
    # immediate forms. The opcode table is F6/F7 /5, 0F AF, and 69/6B.
    out["imul"] = [_enc(0xF6, family="evex", reg=5),
                   _enc(0xF7, family="evex", reg=5),
                   _enc(0xAF, family="evex"),
                   _enc(0x69, family="evex"), _enc(0x6B, family="evex")]
    # `encode_unary_apx` is also the actual APX path for one-operand and
    # NDD unary forms. Extract each mnemonic/op-extension/size dispatch arm
    # rather than guessing from names, then claim the byte and word/dword/qword
    # Group 3 or Group 5 opcode with its ModR/M digit.
    unary = re.compile(
        r'"([a-z0-9]+)"\s*=>\s*self\.encode_unary_rm(?:_66)?'
        r'\(ops,\s*(\d+),\s*(\d+)\)')
    for name, ext_s, size_s in unary.findall(text):
        ext = int(ext_s)
        op = (0xFE if int(size_s) == 1 else 0xFF) if ext <= 1 else (
            0xF6 if int(size_s) == 1 else 0xF7)
        out.setdefault(gas_name(name), []).append(
            _enc(op, family="evex", reg=ext))
    # NDD double shifts keep A5/AD for the CL form and remap immediate A4/AC
    # to 24/2C. The code path is explicit in encode_double_shift(_ndd).
    for name, imm_opcode, cl_opcode in (
        ("shld", 0x24, 0xA5), ("shrd", 0x2C, 0xAD)):
        out.setdefault(name, []).extend((
            _enc(imm_opcode, family="evex"),
            _enc(cl_opcode, family="evex"),
        ))
    # BMI1/BMI2 helpers contain explicit EVEX branches (`avx.rs`): APX EVEX
    # versions retain the instruction's 0F38/0F3A map, pp and opcode, while
    # encoding the wider GPR registers through EVEX. Keep helper parameters
    # adjacent to
    # the source method names so a changed opcode/pp is caught by GAS verify.
    bmi_pp = {0: "none", 1: "66", 2: "f3", 3: "f2"}
    bmi_forms = {
        "andn": [(0xF2, 0, "0f38")],
        "blsi": [(0xF3, 0, "0f38")],
        "blsr": [(0xF3, 0, "0f38")],
        "blsmsk": [(0xF3, 0, "0f38")],
        "bextr": [(0xF7, 0, "0f38")],
        "bzhi": [(0xF5, 0, "0f38")],
        "shlx": [(0xF7, 1, "0f38")],
        "shrx": [(0xF7, 3, "0f38")],
        "sarx": [(0xF7, 2, "0f38")],
        "pext": [(0xF5, 2, "0f38")],
        "pdep": [(0xF5, 3, "0f38")],
        "mulx": [(0xF6, 3, "0f38")],
        # RORX does not use EVexMap4: APX uses EVEX map 0F3A and F2.pp.
        "rorx": [(0xF0, 3, "0f3a")],
    }
    for name, forms in bmi_forms.items():
        out.setdefault(name, []).extend(
            _enc(opcode, family="evex", map_=map_, pp=bmi_pp[pp])
            for opcode, pp, map_ in forms)
    # `encode_div_idiv` has APX branches through `encode_unary_apx`; the
    # ModR/M extension remains /6 or /7 over Group-3 F6/F7. The base opcode
    # varies with byte versus wider operand size, but these helpers dispatch
    # only word/dword/qword `div`/`idiv` spellings here.
    for name, ext in (("div", 6), ("idiv", 7)):
        out.setdefault(name, []).extend((
            _enc(0xF6, family="evex", reg=ext),
            _enc(0xF7, family="evex", reg=ext),
        ))
    # `encode_crc32` drops the legacy F2 prefix for its APX branch. Width is
    # reflected in W/pp but all map-4 forms share F0/F1.
    out.setdefault("crc32", []).extend((
        _enc(0xF0, family="evex"), _enc(0xF1, family="evex"),
    ))
    # `encode_bit_count` has APX map-4 remaps: LZCNT F5, TZCNT F4,
    # POPCNT 88 (with the map carried by the EVEX prefix, not 0F).
    for name, opcode in (("lzcnt", 0xF5), ("tzcnt", 0xF4), ("popcnt", 0x88)):
        out.setdefault(name, []).append(_enc(opcode, family="evex"))
    for name in list(out):
        out[name] = _dedupe({name: out[name]})[name]
    return out


def i686_sse_code_models(
        table: dict[str, list[dict]] | None = None) -> dict[str, list[dict]]:
    """Model the i686 generic SSE helper's literal prefix/opcode dispatch.

    `encode_sse_op` intentionally drops a leading 66 for MMX operands, so an
    SSE2 helper arm with that literal prefix emits both the XMM `66 0F xx`
    and MMX `0F xx` forms. Other leading mandatory prefixes are not dropped.
    The model is sourced from each dispatch's literal byte array and checked
    against the pinned assembler table.
    """
    path = REPO / "src/backend/i686/assembler/encoder/mod.rs"
    text = path.read_text(encoding="utf-8", errors="replace")
    table = table if table is not None else load_snapshot()
    out: dict[str, list[dict]] = {}
    arm = re.compile(
        r'(?P<names>(?:"[a-z0-9]+"(?:\s*\|\s*"[a-z0-9]+")*))'
        r'\s*=>\s*self\.(?P<helper>encode_sse_[a-z0-9_]+)'
        r'\(ops,(?P<args>[^\n]*)')
    arrays = re.compile(r'&\[([^\]]+)\]')
    for match in arm.finditer(text):
        names = QUOTED.findall(match.group("names"))
        helper, args = match.group("helper"), match.group("args")
        raw_arrays = arrays.findall(args)
        if helper == "encode_sse_byte_shift":
            digits = re.findall(r",\s*(\d+)(?:\s*\)|\s*,)", args)
            if not digits:
                continue
            ext = int(digits[0])
            for name in names:
                key = name if name in table else gas_name(name)
                candidates = [_enc(0x73, map_="0f", pp="66", reg=ext)]
                accepted = [e for e in candidates
                            if any(_same(e, row) for row in table.get(key, []))]
                if accepted:
                    out.setdefault(key, []).extend(accepted)
            continue
        parsed = []
        for byte_list in raw_arrays:
            try:
                bs = [int(tok.strip(), 0) for tok in byte_list.split(",")]
            except ValueError:
                continue
            pp = "none"
            if bs and bs[0] in PP_FOR_BYTE:
                pp = PP_FOR_BYTE[bs.pop(0)]
            map_ = "none"
            if bs[:2] == [0x0F, 0x38]:
                map_, bs = "0f38", bs[2:]
            elif bs[:2] == [0x0F, 0x3A]:
                map_, bs = "0f3a", bs[2:]
            elif bs[:1] == [0x0F]:
                map_, bs = "0f", bs[1:]
            if len(bs) == 1:
                parsed.append((bs[0], map_, pp))
        if not parsed:
            continue
        for name in names:
            key = name if name in table else gas_name(name)
            candidates = [_enc(op, map_=map_, pp=pp)
                          for op, map_, pp in parsed]
            if helper in ("encode_sse_op", "encode_sse_op_imm8") \
                    and parsed[0][2] == "66":
                # Both helpers remove a 66 prefix for MMX operands.
                candidates.append(_enc(parsed[0][0], map_=parsed[0][1]))
            if helper == "encode_sse_shift" and len(parsed) > 1:
                ext_match = re.search(
                    r'&\[[^\]]+\]\s*,\s*(\d+)\s*,\s*&\[', args)
                if ext_match:
                    ext = int(ext_match.group(1))
                    op, map_, pp = parsed[1]
                    candidates.append(_enc(op, map_=map_, pp=pp, reg=ext))
                    if parsed[0][2] == "66":
                        # The helper uses the shared MMX prefix-elision logic
                        # for both CL/register and immediate shift encodings.
                        op0, map0, _ = parsed[0]
                        candidates.append(_enc(op0, map_=map0))
                        candidates.append(_enc(op, map_=map_, reg=ext))
            accepted = [e for e in candidates
                        if any(_same(e, row) for row in table.get(key, []))]
            if accepted:
                out.setdefault(key, []).extend(accepted)
    return _dedupe(out)


def x86_special_code_models() -> dict[str, list[dict]]:
    """Model fixed instruction families whose encodings are emitted in code.

    Each entry corresponds to a named encoder path: APX PUSH/POP helpers,
    moffs MOVABS, MOV segment/control-register transfers, and MOVSXD. These
    models deliberately enumerate only bytes present in the implementations;
    the verifier still requires GAS 2.47 to accept every claim.
    """
    out: dict[str, list[dict]] = {}
    # apx.rs: pushp/popp use REX2 plus 50+rd/58+rd; table records the opcode
    # byte after the prefix, with the register range expanded by the snapshot.
    out["pushp"] = [_enc(0x50 + n) for n in range(8)]
    out["popp"] = [_enc(0x58 + n) for n in range(8)]
    # APX push2/pop2 emit EVEX map-4, with W set by the p suffix.
    for name, opcode, digit in (("push2", 0xFF, 6), ("pop2", 0x8F, 0)):
        out[name] = [_enc(opcode, family="evex", reg=digit, w=0)]
        out[name + "p"] = [_enc(opcode, family="evex", reg=digit, w=1)]
    # encode_movabs emits A0..A3 for absolute moffs transfers to/from AL/AX/EAX/RAX.
    out["movabs"] = [_enc(op) for op in range(0xA0, 0xA4)]
    # encode_mov handles segment and control/debug/test registers through
    # 8C/8E and 0F20..26, respectively. The opcodes are explicit in the helper.
    out["mov"] = ([_enc(op, map_="0f") for op in (0x20, 0x21, 0x22, 0x23, 0x24, 0x26)]
                  + [_enc(0x8C), _enc(0x8E)])
    # `encode_movsx`'s dword-to-qword MOVSXD path is opcode 63.
    out["movsx"] = [_enc(0x63)]
    return _dedupe(out)


def evex_direct_code_models() -> dict[str, list[dict]]:
    """Extract directly parameterized EVEX dispatch arms.

    The x86 dispatcher gives each mnemonic's map, pp, W and opcode as literal
    arguments to helpers such as `encode_evex_binary`, `encode_evex_binary_scalar`,
    `encode_evex_unary`, and EVEX immediate forms. Model exactly those
    arguments, not an ISA-family guess. EVEX maps 5 and 6 are represented by
    the opcode table's `none` map (the prefix map value itself has no 0F escape
    byte). The strict GAS snapshot gate catches stale or illegal arms.
    """
    text = (REPO / "src/backend/x86/assembler/encoder/mod.rs").read_text(
        encoding="utf-8", errors="replace")
    arm = re.compile(
        r'"([a-z0-9]+)"\s*=>\s*r?\(self\.encode_evex_\w+'
        r'\(ops,\s*(\d+),\s*(\d+),\s*(\d+),\s*'
        r'(0x[0-9a-fA-F]+|\d+)\)\)')
    maps = {1: "0f", 2: "0f38", 3: "0f3a", 5: "none", 6: "none"}
    pps = {0: "none", 1: "66", 2: "f3", 3: "f2"}
    out: dict[str, list[dict]] = {}
    for name, map_s, pp_s, w_s, opcode_s in arm.findall(text):
        map_n, pp_n = int(map_s), int(pp_s)
        if map_n not in maps or pp_n not in pps:
            continue
        out.setdefault(name, []).append(_enc(
            int(opcode_s, 0), family="evex", map_=maps[map_n],
            pp=pps[pp_n], w=int(w_s)))

    def add(name: str, map_n: int, pp_n: int, w: int, opcode: str | int) -> None:
        if map_n in maps and pp_n in pps:
            out.setdefault(name, []).append(_enc(
                int(opcode, 0) if isinstance(opcode, str) else opcode,
                family="evex", map_=maps[map_n], pp=pps[pp_n], w=w))

    # EVEX helpers with the same encoded map/pp/W/opcode axes but a different
    # operand-shape/SAE contract. The structural shape is enforced by each
    # encoder helper; these models only claim the source-literal opcode tuple.
    for name, ma, pp, w, op, _imm in re.findall(
            r'"([a-z0-9]+)"\s*=>\s*r\(self\.encode_evex_3src_imm'
            r'\(ops,\s*(\d+),\s*(\d+),\s*(\d+),\s*'
            r'(0x[0-9a-fA-F]+|\d+),\s*(true|false)\)\)', text):
        add(name, int(ma), int(pp), int(w), op)
    for name, ma, pp, w, op in re.findall(
            r'"([a-z0-9]+)"\s*=>\s*\{?\s*r\(self\.encode_evex_cmp_mask'
            r'\(ops,\s*mnemonic,\s*(\d+),\s*(\d+),\s*(\d+),\s*'
            r'(0x[0-9a-fA-F]+|\d+)', text):
        add(name, int(ma), int(pp), int(w), op)
    for name, op, w in re.findall(
            r'"([a-z0-9]+)"\s*=>\s*r\(self\.encode_evex_gather'
            r'\(ops,\s*(0x[0-9a-fA-F]+|\d+),\s*(\d+)\)\)', text):
        add(name, 2, 1, int(w), op)
    for name, method, w, op in re.findall(
            r'"([a-z0-9]+)"\s*=>\s*r\(self\.encode_evex_(insert|extract)_lane'
            r'\(ops,\s*mnemonic,\s*(\d+),\s*(0x[0-9a-fA-F]+|\d+)\)\)', text):
        add(name, 3, 1, int(w), op)
    return _dedupe(out)


def code_models(backend: str = "all",
                table: dict[str, list[dict]] | None = None) -> dict[str, list[dict]]:
    """Encodings produced by code paths the row model cannot see."""
    out: dict[str, list[dict]] = {}
    parts = (alu_code_models(), cc_code_models(backend),
             incdec_code_models(backend),
             i686_sse_code_models(table) if backend in ("i686", "all") else {},
             apx_integer_code_models() if backend in ("x86", "all") else {},
             x86_special_code_models() if backend in ("x86", "all") else {},
             evex_direct_code_models() if backend in ("x86", "all") else {},
             fma3_code_models(table) if table is not None else {})
    for part in parts:
        for name, encs in part.items():
            out.setdefault(name, []).extend(encs)
    return _dedupe(out)


def verify_code_models(extra: dict[str, list[dict]],
                       table: dict[str, list[dict]]) -> tuple[list[dict], int]:
    """Check every code-model claim against the snapshot.

    Returns `(findings, checked)`.  A claim the snapshot cannot confirm is a
    finding, in both directions: `CODE-UNKNOWN` when the mnemonic itself is not
    a `as` 2.47 mnemonic, `CODE-EXTRA` when the derived encoding is not one of
    the forms `as` accepts for it.  This is the same contract `LCCC-EXTRA`
    applies to declarative rows, which is why a check on it is worth having.
    """
    findings: list[dict] = []
    checked = 0
    for name in sorted(extra):
        known = table.get(name)
        if known is None:
            findings.append({"kind": "CODE-UNKNOWN", "name": name,
                             "want": extra[name][0]})
            continue
        for enc in extra[name]:
            if any(_same(enc, have) for have in known):
                checked += 1
            else:
                findings.append({"kind": "CODE-EXTRA", "name": name, "want": enc})
    return findings, checked


def missing(rows: list[dict], table: dict[str, list[dict]], model,
            extra: dict[str, list[dict]] | None = None) -> dict[str, list[dict]]:
    """Encodings `as` accepts for mnemonics the encoder already knows."""
    have: dict[str, list[dict]] = {}
    for row in rows:
        # One row can bind mnemonics whose *encodings differ by spelling*:
        # `"lzcntl" | "tzcntl" | "popcntl"` is `0F BD`/`0F BC`/`0F B8` from one
        # arm, and `"rolq" | "roll" | "rolw" | "rolb"` is `D1`/`D0` from
        # another.  Modelling the row once with its first name therefore
        # credited every sibling with the first one's opcode, which `--verify`
        # could not see (the opcode is legal for the *set* of names) and
        # `--missing` reported as two dozen phantom gaps.
        for name in row["names"]:
            encs = model(row, name)
            if encs is None:
                continue
            # XED's catalogue models a `+rd` opcode as one canonical base byte;
            # GAS's row gap report must account for every concrete opcode the
            # encoder can select. Keep this expansion in the GAS comparison
            # layer so it does not distort XED's canonical-row verifier.
            helper = re.findall(r"self\.(encode_[a-z0-9_]+)\(", row["body"])
            helper = helper[-1] if helper else ""
            ranges = {
                "encode_push": (0x50, "none"), "encode_push16": (0x50, "none"),
                "encode_pop": (0x58, "none"), "encode_pop16": (0x58, "none"),
                "encode_bswap": (0xC8, "0f"), "encode_movabs": (0xB8, "none"),
                "encode_pushp": (0x50, "none"), "encode_popp": (0x58, "none"),
            }
            if helper in ranges:
                opcode, map_ = ranges[helper]
                encs = [e for e in encs if not (e["opcode"] == opcode and e["map"] == map_)] + [
                    dict(e, opcode=opcode | rd) for e in encs
                    if e["opcode"] == opcode and e["map"] == map_ for rd in range(8)]
            elif helper == "encode_xchg":
                encs = [e for e in encs if e["opcode"] != 0x90] + [
                    dict(e, opcode=0x90 | rd) for e in encs if e["opcode"] == 0x90
                    for rd in range(8)]
            elif helper == "encode_mov":
                encs = [e for e in encs if e["opcode"] not in (0xB0, 0xB8)] + [
                    dict(e, opcode=e["opcode"] | rd) for e in encs
                    if e["opcode"] in (0xB0, 0xB8) for rd in range(8)]
            elif helper == "encode_inc_dec" and "/i686/" in row["file"]:
                args = re.search(r"encode_inc_dec\(ops,\s*([01]),\s*(\d+)\)", row["body"])
                if args and int(args.group(2)) in (2, 4):
                    opbase = 0x40 if args.group(1) == "0" else 0x48
                    encs.extend(_enc(opbase | rd) for rd in range(8))
            base = gas_name(name)
            have.setdefault(name, []).extend(encs)
            have.setdefault(base, []).extend(encs)
    for name, encs in (extra or {}).items():
        have.setdefault(name, []).extend(encs)
    gaps: dict[str, list[dict]] = {}
    for name, encs in table.items():
        if name not in have:
            continue
        # `cs`/`ds`/`es`/`ss`/`fs`/`gs` are *segment prefixes*, not
        # instructions: `--verify` already skips them, and counting their one
        # byte encodings as "accepted encodings the encoder does not model"
        # made the gap list claim six instructions that do not exist.
        if name in MNEMONIC_FRAGMENTS or name in SNAPSHOT_LIMITATIONS:
            continue
        mine = have[name]
        for enc in encs:
            if not any(_same(enc, m) for m in mine):
                gaps.setdefault(name, []).append(enc)
    return gaps


def unimplemented_mnemonics(rows: list[dict], table: dict[str, list[dict]],
                            extra: dict[str, list[dict]] | None = None) -> list[str]:
    named = set(extra or {})
    for row in rows:
        for name in row["names"]:
            named.add(name)
            named.add(gas_name(name))
    return sorted(n for n in table if n not in named)


def coverage(rows: list[dict], table: dict[str, list[dict]],
             extra: dict[str, list[dict]] | None = None) -> dict[str, list[dict]]:
    """Reference encodings for every mnemonic the encoder has no row for."""
    have = set(extra or {})
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
    # `--missing | head` closes the pipe early; without this the interpreter
    # prints a BrokenPipeError traceback for a perfectly ordinary pipeline.
    if hasattr(signal, "SIGPIPE"):
        signal.signal(signal.SIGPIPE, signal.SIG_DFL)
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
    ap.add_argument("--strict", action="store_true", help="non-zero exit on findings, unmodelled rows or uncovered mnemonics")
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

    code_findings: list[dict] = []
    if args.verify:
        findings, unmodelled, checked, uncovered = verify(rows, table, model)
        # The declarative rows are only half of the encoder: `encode_alu`,
        # `cc_from_mnemonic`, `inc`/`dec` and `fma3_opcode` derive their
        # opcodes in code, so no row exists to check.  `code_models()` reads
        # those code paths and `verify_code_models` requires every claim it
        # makes to be a form `as` 2.47 accepts -- otherwise a model could
        # quietly drift from the encoder and shrink the gap counts instead of
        # failing.
        claims = code_models(args.backend, table)
        code_findings, code_checked = verify_code_models(claims, table)
        print(f"\nrows checked against as 2.47: {checked}")
        print(f"code-model encodings checked: {code_checked} in {len(claims)} mnemonics")
        print(f"findings: {len(findings)}   code-model findings: "
              f"{len(code_findings)}   unmodelled rows: {len(unmodelled)}"
              f"   not in the as 2.47 snapshot: {len(uncovered)}")
        for f in code_findings:
            print(f"  {f['kind']:13s} {f['name']:14s} {_fmt(f['want'])}")
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
        if unmodelled:
            groups: dict[str, int] = {}
            for row in unmodelled:
                helper = HELPER_OF.search(row["body"])
                key = helper.group(1) if helper else row["body"].split("(")[0].strip()
                groups[key] = groups.get(key, 0) + 1
            for key in sorted(groups, key=lambda k: (-groups[k], k)):
                print(f"  UNMODELLED    {groups[key]:5d} rows via {key}")
            for row in unmodelled[: args.limit]:
                print(f"  UNMODELLED    {row['names'][0]:14s} {row['file']}:{row['line']}")
                print(f"                {row['body'][:110]}")
        if uncovered:
            names = sorted({u["name"] for u in uncovered})
            print(f"  UNCOVERED     {len(names)} mnemonic(s) with no entry in the "
                  f"as 2.47 snapshot: {' '.join(names)}")
        if args.strict and (findings or code_findings or unmodelled or uncovered):
            return 1

    if args.missing:
        gaps = missing(rows, table, model, code_models(args.backend, table))
        total = sum(len(v) for v in gaps.values())
        print(f"\nencodings as 2.47 accepts that no declarative row models: {total} "
              f"across {len(gaps)} mnemonics with at least one row "
              f"(code-implemented families are not rows; see the docstring)")
        for name in sorted(gaps):
            forms = ", ".join(_fmt(e) for e in gaps[name])
            print(f"  {name:14s} {forms}")

    if args.coverage:
        gap = coverage(rows, table, code_models(args.backend, table))
        Path(args.coverage).write_text(json.dumps(gap, indent=1, sort_keys=True) + "\n")
        legal = sum(len(v) for v in gap.values())
        print(f"\nmnemonics with no declarative row (written to {args.coverage}):"
              f" {len(gap)} carrying {legal} reference encodings")

    if args.absent:
        absent = unimplemented_mnemonics(rows, table, code_models(args.backend, table))
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
