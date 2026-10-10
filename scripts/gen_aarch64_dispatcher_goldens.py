#!/usr/bin/env python3
"""Generate tests/aarch64/dispatcher-goldens.tsv — the dispatcher ratchet.

WHY
---
``aarch64_encoder_differential.py`` samples the encoding space uniformly at
random, so a rare form (an indexed element load, an LSE pairing, a system
op) can hide from it for a long time, and its sweep is a manual offline run.
The dispatcher in ``encoder/mod.rs`` has ~401 mnemonic arms; a routing bug
in one of them (wrong leaf, wrong operand transformation — the shape in
which an ``fmls`` defect once hid) must be caught by the *default* test
run with no cross binutils and no network.

This script produces the deterministic half of that contract:

1. Enumerate the dispatcher arms mechanically from ``encoder/mod.rs``
   (no hand-maintained mnemonic list — a new arm cannot dodge the ratchet).
2. For every arm, pick one representative instruction:
   * candidates from the random-word differential corpus (objdump-canonical
     text, i.e. exactly what binutils itself calls the encoding), else
   * a hand template from HAND_TEMPLATES below (aliases, system ops and
     hint/barrier forms objdump rarely or never prints).
3. Assemble the candidate with GNU as 2.47 (golden) and with LCCC; keep
   the first row where both accept and agree — the golden word is GAS's,
   never ours.
4. Write the TSV sorted by mnemonic.  ``--check`` re-validates the file
   against GNU as without rewriting it.

The consuming ratchet lives in ``encoder/mod.rs`` (test module
``dispatcher_goldens``): it re-derives the arm set at test time, requires
every arm to be covered, and re-assembles every row through the full
parser→dispatcher→leaf path, so a drifted table or a new unrouted arm
fails ``cargo test --lib`` on any host.

USAGE
-----
    scripts/gen_aarch64_dispatcher_goldens.py                 # regenerate
    scripts/gen_aarch64_dispatcher_goldens.py --check         # verify rows
"""
from __future__ import annotations

import argparse
import re
import sys
import types
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO / "scripts"))
import aarch64_encoder_differential as D  # noqa: E402

DISPATCHER = REPO / "src/backend/arm/assembler/encoder/mod.rs"
TSV = REPO / "tests/aarch64/dispatcher-goldens.tsv"

# The pinned architectural law (scripts/aarch64_operand_legality_matrix.py
# PROLOGUE) plus the AES/SHA3 feature extensions GAS's `.arch armv9.4-a`
# mapping does not imply by itself (measured: `aese v0.16b, v1.16b` and
# `eor3 ...` assemble only with the explicit flags).  Encoding words are
# flag-independent -- the flags only widen what as accepts -- so one
# prologue gives every candidate the same verdict surface lccc must meet.
PROLOGUE = ".arch armv9.4-a+sme+aes+sha3\n.text\n"

# Representative instructions for arms the random-word corpus cannot reach
# (objdump prints aliases or never decodes the form).  Every template is
# validated against GNU as at generation time — a wrong one is reported,
# not silently baked.  Keyed by DISPATCHER ARM, so the text may differ from
# the key when the arm is an alias router (ldrw → `ldr w0, [x1]`).
HAND_TEMPLATES: dict[str, list[str]] = {
    "abs": ["abs v0.16b, v1.16b"],
    "addv": ["addv b0, v0.8b"],
    "adrp": ["adrp x0, foo"],
    "aesd": ["aesd v0.16b, v1.16b"],
    "aese": ["aese v0.16b, v1.16b"],
    "aesimc": ["aesimc v0.16b, v1.16b"],
    "aesmc": ["aesmc v0.16b, v1.16b"],
    "at": ["at s1e1r, x0"],
    "bfm": ["bfm x0, x1, #1, #2"],
    "blr": ["blr x0"],
    "br": ["br x0"],
    "bti": ["bti"],
    "cas": ["cas w1, w2, [x0]"],
    "casp": ["casp w0, w1, w2, w3, [x4]"],
    "cinv": ["cinv x0, x1, eq"],
    "clrex": ["clrex"],
    "clz": ["clz w1, w0"],
    "cmle": ["cmle v0.8h, v1.8h, #0"],
    "cmlt": ["cmlt v0.4s, v1.4s, #0"],
    "cnt": ["cnt v0.16b, v1.16b"],
    "crc32b": ["crc32b w1, w2, w3"],
    "cset": ["cset x0, eq"],
    "csetm": ["csetm x0, eq"],
    "dc": ["dc civac, x0"],
    "dmb": ["dmb sy"],
    "dsb": ["dsb sy"],
    "eor3": ["eor3 v0.16b, v1.16b, v2.16b, v3.16b"],
    "eret": ["eret"],
    "fcmle": ["fcmle v0.4s, v1.4s, #0"],
    "fcmlt": ["fcmlt v0.4s, v1.4s, #0"],
    "fcmpe": ["fcmpe d0, d1", "fcmpe d0, #0.0"],
    "fcvt": ["fcvt d0, s0"],
    "fcvtas": ["fcvtas s0, s1"],
    "fcvtau": ["fcvtau s0, s1"],
    "fcvtl": ["fcvtl v0.2d, v0.2s"],
    "fcvtl2": ["fcvtl2 v0.2d, v0.4s"],
    "fcvtn": ["fcvtn v0.2s, v1.2d"],
    "fcvtns": ["fcvtns s0, s1"],
    "fcvtnu": ["fcvtnu v0.4s, v1.4s"],
    "fcvtpu": ["fcvtpu d0, d1"],
    "fmaxnmv": ["fmaxnmv s0, v1.4s"],
    "fmaxv": ["fmaxv s0, v1.4s"],
    "fminnmv": ["fminnmv s0, v1.4s"],
    "fminv": ["fminv s0, v1.4s"],
    "fneg": ["fneg d0, d1", "fneg v0.4s, v1.4s"],
    "frinta": ["frinta d0, d1"],
    "frinti": ["frinti s0, s1"],
    "frintm": ["frintm v0.4s, v1.4s"],
    "frintx": ["frintx v0.4s, v1.4s"],
    "frintz": ["frintz d0, d1"],
    "fsqrt": ["fsqrt d0, d1"],
    "hint": ["hint #0"],
    "ic": ["ic ivau, x0"],
    "ins": ["ins v0.b[0], v1.b[0]"],
    "isb": ["isb sy"],
    "ldadd": ["ldadd x1, x2, [x3]"],
    "ldar": ["ldar x0, [x1]"],
    "ldarb": ["ldarb w0, [x1]"],
    "ldarh": ["ldarh w0, [x1]"],
    "ldrw": ["ldr w0, [x1]"],
    "movn": ["movn x0, #0"],
    "movz": ["movz x0, #0"],
    "nop": ["nop"],
    "not": ["not v0.16b, v0.16b"],
    "rbit": ["rbit w1, w0", "rbit x1, x0"],
    "ret": ["ret"],
    "rev": ["rev x0, x1"],
    "rev16": ["rev16 w0, w1"],
    "rev32": ["rev32 x0, x1"],
    "rev64": ["rev64 v0.16b, v1.16b"],
    "sadalp": ["sadalp v0.4s, v0.8h"],
    "saddlp": ["saddlp v0.8h, v0.16b"],
    "saddlv": ["saddlv s0, v0.8h"],
    "sbfm": ["sbfm x0, x1, #1, #2"],
    "sev": ["sev"],
    "sevl": ["sevl"],
    "shll": ["shll v0.4s, v0.4h, #16", "shll v0.8h, v0.8b, #8"],
    "shll2": ["shll2 v0.4s, v0.8h, #16"],
    "smaxv": ["smaxv b0, v0.8b"],
    "sminv": ["sminv b0, v1.16b"],
    "sqabs": ["sqabs v0.16b, v1.16b"],
    "sqxtn": ["sqxtn v0.4h, v0.4s"],
    "sqxtn2": ["sqxtn2 v0.8h, v0.4s"],
    "sqxtun": ["sqxtun v0.8b, v0.8h"],
    "sqxtun2": ["sqxtun2 v0.16b, v0.8h"],
    "stadd": ["stadd x0, [x1]"],
    "swp": ["swp x1, x2, [x3]"],
    "sxtb": ["sxtb w1, w0"],
    "sxtl": ["sxtl v0.4s, v0.4h"],
    "sxtl2": ["sxtl2 v0.4s, v0.8h"],
    "sxtw": ["sxtw x1, w0"],
    "tlbi": ["tlbi vmalle1"],
    "uaddlv": ["uaddlv d0, v0.4s"],
    "ubfm": ["ubfm x0, x1, #1, #2"],
    "umaxv": ["umaxv s0, v1.4s"],
    "uminv": ["uminv s0, v1.4s"],
    "uqxtn": ["uqxtn v0.4h, v0.4s"],
    "uqxtn2": ["uqxtn2 v0.8h, v0.4s"],
    "uxtb": ["uxtb w1, w0"],
    "uxth": ["uxth w0, w1"],
    "uxtl": ["uxtl v0.4s, v0.4h"],
    "uxtl2": ["uxtl2 v0.4s, v0.8h"],
    "uxtw": ["uxtw x1, w0"],
    "wfe": ["wfe"],
    "wfi": ["wfi"],
    "xtn": ["xtn v0.4h, v0.4s"],
    "xtn2": ["xtn2 v0.8h, v0.4s"],
    "yield": ["yield"],
}


def arm_head_re() -> re.Pattern[str]:
    r"""One arm head: quoted keys starting at the dispatcher's own indent.

    Inner `match`es live deeper, so anchoring on indentation rather than on
    `^\s*` is what keeps a leaf's `if mnemonic == "..."` from being read as an
    arm.  Exported for the generator only -- the Rust ratchet re-implements the
    same rule, and the `# arms: N` line this script writes into the TSV is the
    latch that catches the two copies drifting apart.
    """
    return re.compile(r'^ {8}"[a-z0-9_.]+"', re.M)


def dispatcher_arms() -> list[str]:
    """Mnemonic arms of `encode_instruction`, mechanically extracted.

    Every key in an arm's pattern list counts, not just the first: an arm head
    is a run of `"..." | "..."` groups terminated by `=>`.  The first version
    matched one name per line, so of the dispatcher's multi-pattern arms --
    `cas*`, `casp*`, `swp*`, `ldadd*`/`ldclr*`/`ldeor*`/`ldset*`/`ld*max*`/
    `ld*min*`, `stadd*`/`stclr*`/`steor*`/`stset*`, `crc32*`, `ldrsw`, the
    rounding `fcvt*` group -- it saw the leading mnemonic and silently dropped
    the rest: 423 of 526 arms, with 103 names never even attempted.

    That is the exact family where a routing defect hides.  An alias arm
    forwards to a shared leaf *with the mnemonic string* (`encode_cas(m, ops)`),
    so the leaf re-reads the decoration: a lost `b`/`h` suffix changes the
    element size and register width, a mis-read `al` changes the memory order,
    and a `st*` form that should take `Rt, [Rn]` but reads three operands is
    invisible to every other test in the tree.  An ratchet that prints a
    satisfied count over a hole this large is worse than no ratchet, because it
    certifies the hole.
    """
    src = DISPATCHER.read_text()
    a = src.find("pub fn encode_instruction(")
    if a < 0:
        raise SystemExit("encode_instruction not found in encoder/mod.rs")
    b = src.find("\npub fn ", a + 10)
    body = src[a:b if b > 0 else len(src)]
    lines = body.splitlines()
    head = arm_head_re()
    names: set[str] = set()
    i = 0
    while i < len(lines):
        if not head.match(lines[i]):
            i += 1
            continue
        while True:
            names.update(re.findall(r'"([a-z0-9_.]+)"', lines[i]))
            if "=>" in lines[i]:
                break
            i += 1
            if i >= len(lines) or not lines[i].strip().startswith("|"):
                break
        i += 1
    return sorted(names)


def alias_family_templates(arms: set[str]) -> dict[str, list[str]]:
    """Candidate texts for alias-family arms no random word reaches.

    The LSE atomics and `crc32` occupy slices of the 32-bit encoding space a
    uniform sweep of the corpus misses entirely, so their goldens can only come
    from a template.  Spelling is derived from the mnemonic instead of written
    out variant by variant: the leading verb fixes the operand shape (atomic
    stores take `Rt, [Rn]`, the load-compare-swap forms take `Rt, Rt2, [Rn]`),
    an `a`/`l`/`al` infix is the acquire/release decoration, and a trailing
    `b`/`h` picks the element size and therefore the register width.

    Where a width rule is not certain (`crc32`'s Rt/Rn pair), every plausible
    spelling is offered and GNU as decides: the generator keeps the first
    candidate the oracle accepts, so an over-eager template costs one rejected
    probe rather than writing a wrong golden.  Letting the oracle pick is the
    only honest way for this script to invent a spelling at all.
    """
    out: dict[str, list[str]] = {}
    load_form = "{r0}, {r1}, [x2]"
    store_form = "{r0}, [x2]"
    # The Armv8.1 LSE set: CAS/CASP/SWP, the eight LD* and eight ST* atomics,
    # each optionally decorated with A (acquire), L (release) and B/H (width).
    verbs = {
        "casp": "x0, x1, x2, x3, [x4]",
        "cas": load_form, "swp": load_form,
        "ldadd": load_form, "ldclr": load_form, "ldeor": load_form,
        "ldset": load_form, "ldsmax": load_form, "ldsmin": load_form,
        "ldumax": load_form, "ldumin": load_form,
        "stadd": store_form, "stclr": store_form, "steor": store_form,
        "stset": store_form, "stsmax": store_form, "stsmin": store_form,
        "stumax": store_form, "stumin": store_form,
    }
    by_len = sorted(verbs, key=len, reverse=True)  # `casp` before `cas`
    for mn in sorted(arms):
        if mn in HAND_TEMPLATES:
            continue
        for verb in by_len:
            if not mn.startswith(verb):
                continue
            deco = mn[len(verb):]
            size = ""
            if deco and deco[-1] in "bh":
                size, deco = deco[-1], deco[:-1]
            if deco not in ("", "a", "l", "al"):
                break  # not this family after all
            r0, r1 = ("w0", "w1") if size else ("x0", "x1")
            out.setdefault(mn, []).append(
                f"{mn} " + verbs[verb].format(r0=r0, r1=r1)
            )
            break
        else:
            if mn.startswith("crc32"):
                out.setdefault(mn, []).extend(
                    f"{mn} {a}, {b}, {c}"
                    for a, b, c in (("w0", "w1", "w2"), ("x0", "w1", "w2"),
                                    ("x0", "x1", "w2"), ("w0", "x1", "w2"))
                )
            elif mn.startswith(("ldrsw", "ldrw")):
                out.setdefault(mn, []).extend([f"{mn} x0, [x1]",
                                               f"{mn} x0, [x1, #8]"])
            elif mn.startswith("fcvt"):
                out.setdefault(mn, []).extend(
                    [f"{mn} w0, s1", f"{mn} x0, d1", f"{mn} v0.4s, v1.4s",
                     f"{mn} v0.2d, v1.2d"]
                )
    return out


def build_toolchain(ns: argparse.Namespace) -> D.Toolchain:
    return D.Toolchain.build(types.SimpleNamespace(
        triple="aarch64", gas=ns.gas, objcopy=ns.objcopy, objdump=ns.objdump,
        lccc=ns.lccc, objdump_arch="aarch64", prologue=PROLOGUE))


def gas_word(tc: D.Toolchain, text: str) -> tuple[bool, bytes]:
    r = tc.assemble(text, "gas", use_lccc=False)
    return r.ok, r.data


def lccc_word(tc: D.Toolchain, text: str) -> tuple[bool, bytes]:
    r = tc.assemble(text, "lccc", use_lccc=True)
    return r.ok, r.data


def generate(ns: argparse.Namespace) -> int:
    arms = dispatcher_arms()
    tc = build_toolchain(ns)
    problems: list[str] = []
    try:
        corpus = D.generate_corpus(tc, ns.n, ns.seed)
        by_mnemonic: dict[str, list[D.Insn]] = {}
        for ins in corpus:
            by_mnemonic.setdefault(ins.mnemonic, []).append(ins)

        extra = alias_family_templates(set(arms))
        rows: dict[str, tuple[str, str]] = {}
        mismatches: list[str] = []
        for arm in arms:
            candidates: list[str] = []
            if arm in by_mnemonic:
                # Deterministic: sorted canonical texts, first agreement wins.
                candidates.extend(
                    sorted({i.text for i in by_mnemonic[arm]})
                )
            candidates.extend(HAND_TEMPLATES.get(arm, []))
            candidates.extend(extra.get(arm, []))
            if not candidates:
                problems.append(f"{arm}: no corpus row and no hand template")
                continue
            any_gas_ok = False
            for text in candidates:
                gok, gdata = gas_word(tc, text)
                if not gok:
                    continue  # alias not accepted in this spelling; next
                any_gas_ok = True
                lok, ldata = lccc_word(tc, text)
                if not lok:
                    mismatches.append(f"{arm}: LCCC rejects `{text}` "
                                      f"(GAS -> {gdata.hex()})")
                    continue
                if ldata != gdata:
                    mismatches.append(
                        f"{arm}: `{text}` GAS {gdata.hex()} != "
                        f"LCCC {ldata.hex()}")
                    continue
                rows[arm] = (text, gdata.hex())
                break
            if arm not in rows and not any_gas_ok:
                problems.append(f"{arm}: GAS rejected every candidate (tried: "
                                + "; ".join(candidates) + ")")
        # staleness: templates for arms that no longer exist
        for arm in sorted(HAND_TEMPLATES):
            if arm not in arms:
                problems.append(f"hand template for unknown arm `{arm}`")

        # Ratchet invariant: the TSV is only written when EVERY dispatcher
        # arm carries at least one row both toolchains agree on.  Skipped
        # candidate mismatches are diagnostics (the loop already moved on to
        # the next spelling); uncovered arms are the real gate.
        missing = sorted(set(arms) - set(rows))
        if mismatches:
            print("LCCC/GAS candidate mismatches (diagnostic, skipped):",
                  file=sys.stderr)
            for m in mismatches[:40]:
                print("  " + m, file=sys.stderr)
            if len(mismatches) > 40:
                print(f"  ... and {len(mismatches) - 40} more", file=sys.stderr)
        if problems:
            print("coverage problems:", file=sys.stderr)
            for p in problems:
                print("  " + p, file=sys.stderr)
        if missing:
            print(
                f"UNCOVERED {len(missing)}/{len(arms)} arms (no agreed row): "
                + ", ".join(missing),
                file=sys.stderr,
            )
            return 1

        lines = [
            "# AArch64 dispatcher goldens -- one GAS-verified instruction per",
            "# encode_instruction() arm in src/backend/arm/assembler/encoder/mod.rs.",
            "# Generated by scripts/gen_aarch64_dispatcher_goldens.py; do not",
            "# hand-edit.  The ratchet test (encoder/mod.rs dispatcher_goldens)",
            "# re-derives the arm set and re-assembles every row through the full",
            "# parser->dispatcher->leaf path, so a new arm without a row, or a",
            "# drifted word, fails cargo test --lib on a host with no cross tools.",
            f"# arms: {len(arms)} -- arm count as enumerated by this script;",
            "# the ratchet test compares its own re-derivation against it, so",
            "# the two copies of the enumeration rule cannot drift apart.",
            "# mnemonic\tinstruction\tencoding (GNU as 2.47, LE hex)",
        ]
        for arm in sorted(rows):
            text, word = rows[arm]
            # Corpus texts may carry a trailing tab+comment (`ccmn ... cs
            # \t// cs = hs, nlast`); comments are assemblable but tabs
            # would break the TSV's 3-field contract, so keep the
            # instruction alone.
            text = text.split("\t")[0].strip()
            lines.append(f"{arm}\t{text}\t{word}")
        TSV.parent.mkdir(parents=True, exist_ok=True)
        tmp = TSV.with_suffix(".tsv.tmp")
        tmp.write_text("\n".join(lines) + "\n")
        tmp.replace(TSV)
        print(f"wrote {TSV} ({len(rows)}/{len(arms)} arms, corpus {len(corpus)})")
        return 0
    finally:
        tc.close()


def check(ns: argparse.Namespace) -> int:
    """Re-validate every baked row against GNU as (and the arm set)."""
    arms = set(dispatcher_arms())
    rows = []
    for line in TSV.read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        mnemonic, text, word = line.split("\t")
        rows.append((mnemonic, text, word))
    bad = 0
    for mnemonic, text, word in rows:
        if mnemonic not in arms:
            print(f"stale row: `{mnemonic}` is no longer a dispatcher arm")
            bad += 1
    tc = build_toolchain(ns)
    try:
        for mnemonic, text, word in rows:
            gok, gdata = gas_word(tc, text)
            if not gok or gdata.hex() != word:
                print(f"drift: `{text}` expected {word}, GAS says "
                      f"{gdata.hex() if gok else 'REJECT'}")
                bad += 1
    finally:
        tc.close()
    missing = sorted(arms - {m for m, _, _ in rows})
    for m in missing:
        print(f"uncovered dispatcher arm: `{m}`")
        bad += 1
    if bad:
        print(f"{bad} problem(s)")
        return 1
    print(f"OK: {len(rows)} rows match GNU as; all {len(arms)} arms covered")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--n", type=int, default=200_000)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--gas", default="/home/user/.cache/gas-2.47-aarch64-linux-gnu/bin/as")
    ap.add_argument("--objcopy", default="/home/user/.cache/gas-2.47-aarch64-linux-gnu/bin/objcopy")
    ap.add_argument("--objdump", default="/home/user/.cache/gas-2.47-aarch64-linux-gnu/bin/objdump")
    ap.add_argument("--lccc", default=str(REPO / "target/fastbuild/lccc"))
    ns = ap.parse_args()
    return check(ns) if ns.check else generate(ns)


if __name__ == "__main__":
    sys.exit(main())
