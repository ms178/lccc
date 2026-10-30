#!/usr/bin/env python3
"""Cross-target NOBITS and `.org` parity with GNU as, pinned.

Every row is assembled by lccc for x86-64, i686, AArch64 and RISC-V and its
outcome compared with the GAS verdict recorded below: the `.bss` sh_size (or
an error) for the NOBITS rows, the section bytes (or an error) for the `.org`
rows. The expectations were measured with GNU as 2.47 (x86-64, i686 via
`--32`, AArch64) and Debian's GNU as 2.44 for RISC-V (`-march=rv64gc
-mno-relax`); re-measure with `--oracle` (see below) instead of editing them.

Why this gate exists: PR #844 made zero runs in x86 `.bss` a size but left
AArch64/RISC-V refusing `char big[1 << 30];` ("fill directive ... exceeds
the 268435456-byte limit"), kept three parse-time `.fill` caps that refused
the very NOBITS case its writer accepted, and turned `.bss; .zero 16; .skip
2f-1f` into an internal error. Separately, RISC-V dropped every `.org`
(`.byte 1; .org 8; .byte 2` assembled to `01 02`) and AArch64 refused the
kernel's ALTERNATIVE length check. Its x86-only sibling gate
(check_sparse_bss_nobits.sh) could see none of that.

DIVERGENCES are deliberate, each with its reason; every other row must match.

Usage:
  check_nobits_org_parity.py --lccc target/fastbuild/lccc
      (lccc-i686, lccc-arm and lccc-riscv are taken from the same directory)
  check_nobits_org_parity.py --lccc ... --oracle x86_64=AS --oracle aarch64=AS \
      --oracle riscv64=AS
      re-measure the pinned table against real assemblers and report drift.
"""

import argparse
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from elf_probe import Elf  # noqa: E402

G = 1 << 30

# NOBITS rows: `.bss; .byte 0; FORM; after: .long 0`.
NOBITS_FORMS = [
    ".long 0", ".quad 0", ".zero 5", ".skip 3,1", ".space 5,9", ".fill 3,4,0",
    ".fill 3,4,1", ".p2align 3", ".p2align 4,0x90", ".balign 8", ".org 64", ".org 64,7",
    ".uleb128 0", ".sleb128 0", ".uleb128 1", ".byte 1", '.ascii ""', '.ascii "a"',
    f".zero {G}", f".skip {G}", f".fill {G},1,0", f".space {G}", f".org {G}",
    f".zero {16 * G}", f".fill {4 * G},4,0", ".p2align 30",
    ".subsection 1\n.zero 8\n.subsection 0\n.zero 4",
    "a1: .long b1 - a1\nb1:", "a2: .long b2 - a2 - 4\nb2:",
    ".zero 16\n.skip 2f-1f\n1: .zero 4\n2:",
]

# GAS verdict per form: the .bss size, or None for an error. One column per
# target where they differ (i686 wraps 16 GiB; LEB is fail-closed on arm/rv).
ALL = ("x86_64", "i686", "aarch64", "riscv64")
NOBITS_EXPECT = {
    ".long 0": 9, ".quad 0": 13, ".zero 5": 10, ".skip 3,1": 8, ".space 5,9": 10,
    ".fill 3,4,0": 17, ".fill 3,4,1": None, ".p2align 3": 12, ".p2align 4,0x90": 20,
    ".balign 8": 12, ".org 64": 0x44, ".org 64,7": 0x44, ".uleb128 0": 6,
    ".sleb128 0": 6, ".uleb128 1": None, ".byte 1": None, '.ascii ""': 5,
    '.ascii "a"': None, f".zero {G}": G + 5, f".skip {G}": G + 5,
    f".fill {G},1,0": G + 5, f".space {G}": G + 5, f".org {G}": G + 4,
    f".zero {16 * G}": 16 * G + 5, f".fill {4 * G},4,0": 16 * G + 5,
    ".p2align 30": G + 4,
    ".subsection 1\n.zero 8\n.subsection 0\n.zero 4": 17,
    # A label difference resolved after the store is a fixup, and GAS
    # refuses any fixup in NOBITS -- even one that evaluates to zero.
    "a1: .long b1 - a1\nb1:": None, "a2: .long b2 - a2 - 4\nb2:": None,
    ".zero 16\n.skip 2f-1f\n1: .zero 4\n2:": 29,
}

# (target, form) -> (lccc's verdict, reason). None = error expected.
DIVERGENCES = {
    ("i686", f".zero {16 * G}"): (None, "GAS wraps the ELF32 size mod 2^32 to a 5-byte .bss; "
                                       "lccc refuses (\"exceeds the ELF32 size limit\") instead of truncating"),
    ("i686", f".fill {4 * G},4,0"): (None, "same ELF32 wrap"),
    ("aarch64", ".uleb128 1"): (None, "fail-closed LEB128 (GAS also errors here)"),
    ("riscv64", ".uleb128 1"): (None, "fail-closed LEB128 (GAS also errors here)"),
    ("aarch64", ".zero 16\n.skip 2f-1f\n1: .zero 4\n2:"): (
        None, "a forward-referenced .skip size is refused on AArch64/RISC-V (immediate "
              "evaluation; x86 defers it)"),
    ("riscv64", ".zero 16\n.skip 2f-1f\n1: .zero 4\n2:"): (None, "same forward .skip gap"),
    ("x86_64", ".subsection 1\n.zero 8\n.subsection 0\n.zero 4"): (
        None, "x86 refuses .subsection: it used to be silently ignored, which is right for a "
              ".bss size but places subsection-1 code inline in .text/.data; ordering it needs "
              "cross-subsection jump relaxation, and no x86 producer emits it (FOLLOWUP)"),
    ("i686", ".subsection 1\n.zero 8\n.subsection 0\n.zero 4"): (None, "same x86 .subsection refusal"),
}

# i686 rows whose GAS verdict is the ELF32 wrap of the x86-64 size, so the
# x86-64 pin does not describe GAS --32 there.
ELF32_WRAP_ROWS = {("i686", f".zero {16 * G}"), ("i686", f".fill {4 * G},4,0")}


def alternative(orig, repl):
    """The kernel ALTERNATIVE length check (arm64/riscv alternative-macros.h)."""
    return (".text\n661:\n" + "\t.word 0x11111111\n" * orig + "662:\n"
            ".pushsection .altinstr_replacement,\"a\"\n663:\n" + "\t.word 0x22222222\n" * repl +
            "664:\n.popsection\n.org . - (664b-663b) + (662b-661b)\n"
            ".org . - (662b-661b) + (664b-663b)\n\t.word 0x33333333\n")


# `.org` rows (AArch64 and RISC-V): name -> (source, {section: hex} or None).
ORG_ROWS = {
    "data_abs": (".data\n.byte 1\n.org 8\n.byte 2\n", {".data": "010000000000000002"}),
    "data_fill": (".data\n.byte 1\n.org 8, 0xaa\n.byte 2\n", {".data": "01aaaaaaaaaaaaaa02"}),
    "dot_rel": (".data\n.byte 1\n.org . + 5\n.byte 2\n", {".data": "01000000000002"}),
    "label_rel": (".data\nL0:\t.byte 1\n.org L0 + 16\n.byte 2\n",
                  {".data": "01" + "00" * 15 + "02"}),
    "set_sym": (".set N, 12\n.data\n.byte 1\n.org N\n.byte 2\n", {".data": "01" + "00" * 11 + "02"}),
    "lead_org": (".org 4\n.word 0x12345678\n", {".text": "0000000078563412"}),
    "text": (".text\n.word 0x01020304\n.org 12\n.word 0x05060708\n",
             {".text": "04030201000000000000000008070605"}),
    "two_x": (".data\nL1:\t.byte 1\n.org L1 * 2 + 4\n", {".data": "01000000"}),
    "back": (".data\n.quad 1\n.org 4\n", None),
    "fwd": (".data\n.org L9\n.byte 1\nL9:\t.byte 2\n", None),
    "other_sec": (".text\nT0:\t.word 1\n.data\n.org T0 + 4\n", None),
    "alt_equal": (alternative(2, 2), {".text": "111111111111111133333333",
                                      ".altinstr_replacement": "2222222222222222"}),
    "alt_short": (alternative(3, 1), None),
    "alt_long": (alternative(1, 3), None),
}


def sections(path):
    """name -> (sh_type, bytes or None for NOBITS, sh_size)."""
    return {name: sec[:3] for name, sec in Elf(path).sections.items()}


def assemble(cmd, src, workdir, tag):
    s = os.path.join(workdir, tag + ".s")
    o = os.path.join(workdir, tag + ".o")
    with open(s, "w") as f:
        f.write(src)
    try:
        r = subprocess.run(cmd + [s, "-o", o], capture_output=True, text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return "timeout", None
    if r.returncode != 0:
        if r.returncode < 0 or "internal error" in r.stderr or "panicked" in r.stderr:
            return "crash", r.stderr.strip()[-200:]
        return "error", None
    return "ok", sections(o)


def nobits_outcome(cmd, form, workdir, tag):
    status, secs = assemble(cmd, f"\t.bss\n\t.byte 0\n{form}\nafter:\n\t.long 0\n", workdir, tag)
    if status != "ok":
        return status if status != "error" else None
    return secs[".bss"][2] if ".bss" in secs else 0


def org_outcome(cmd, src, workdir, tag):
    status, secs = assemble(cmd, src, workdir, tag)
    if status != "ok":
        return status if status != "error" else None
    return {k: (v[1] or b"").hex() for k, v in secs.items()
            if k in (".text", ".data", ".altinstr_replacement") and v[2]}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc", required=True)
    # One pair per occurrence: a greedy `nargs="*"` option would swallow any
    # token that follows it (and trips the corpus ratchet against star
    # arguments parsed with plain parse_args).
    ap.add_argument("--oracle", action="append", default=None,
                    metavar="TARGET=ASSEMBLER",
                    help="re-measure the pinned table against this assembler "
                         "(repeat per target: x86_64, aarch64, riscv64)")
    args = ap.parse_args()
    bindir = os.path.dirname(os.path.abspath(args.lccc))
    lccc = {"x86_64": [args.lccc, "-c"], "i686": [os.path.join(bindir, "lccc-i686"), "-c"],
            "aarch64": [os.path.join(bindir, "lccc-arm"), "-c"],
            "riscv64": [os.path.join(bindir, "lccc-riscv"), "-c"]}
    for tgt, cmd in lccc.items():
        if not os.access(cmd[0], os.X_OK):
            print(f"FAIL: missing {tgt} compiler {cmd[0]}")
            return 1
    failures = 0
    rows = 0
    with tempfile.TemporaryDirectory() as td:
        if args.oracle is not None:
            gas = {}
            for pair in args.oracle:
                tgt, path = pair.split("=", 1)
                gas[tgt] = {"x86_64": [path, "--64"], "aarch64": [path],
                            "riscv64": [path, "-march=rv64gc", "-mno-relax"]}[tgt]
            if "x86_64" in gas:
                gas["i686"] = [gas["x86_64"][0], "--32"]
            drift = 0
            for tgt, cmd in gas.items():
                for form in NOBITS_FORMS:
                    got = nobits_outcome(cmd, form, td, "g")
                    # The pinned GAS verdict must be what GAS says; a
                    # divergence row pins lccc's verdict separately, so the
                    # GAS column is checked for those rows too -- except the
                    # i686 wrap rows, whose GAS column is the x86-64 value.
                    want = NOBITS_EXPECT[form]
                    if got != want and (tgt, form) not in ELF32_WRAP_ROWS:
                        print(f"ORACLE {tgt} {form!r}: GAS {got}, pinned {want}")
                        drift += 1
                if tgt in ("aarch64", "riscv64"):
                    for name, (src, want) in ORG_ROWS.items():
                        got = org_outcome(cmd, src, td, "g")
                        if got != want:
                            print(f"ORACLE {tgt} .org {name}: GAS {got}, pinned {want}")
                            drift += 1
            print(f"oracle: {drift} unexplained difference(s)")
        for tgt in ALL:
            for form in NOBITS_FORMS:
                want, why = DIVERGENCES.get((tgt, form), (NOBITS_EXPECT[form], None))
                got = nobits_outcome(lccc[tgt], form, td, "l")
                rows += 1
                if got != want:
                    failures += 1
                    print(f"FAIL {tgt} nobits {form!r}: lccc {got}, want {want}"
                          + (f" [divergence: {why}]" if why else ""))
        for tgt in ("aarch64", "riscv64"):
            for name, (src, want) in ORG_ROWS.items():
                got = org_outcome(lccc[tgt], src, td, "l")
                rows += 1
                if got != want:
                    failures += 1
                    print(f"FAIL {tgt} .org {name}: lccc {got}, want {want}")
    # The x86 setup/real-mode fixed-slot case also needs byte parity after
    # jump relaxation and a fail-closed backwards-location verdict. Keep it
    # in this existing hosted + local gate, rather than an orphan script.
    x86_test = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                            "check_x86_location_assignment.sh")
    env = {**os.environ, "CCC": os.path.abspath(args.lccc)}
    result = subprocess.run(["bash", x86_test], env=env, check=False)
    rows += 1
    if result.returncode:
        print("FAIL x86 location-counter assignment parity")
        failures += 1
    print(f"nobits/org parity: {rows - failures}/{rows} rows hold "
          f"({len(DIVERGENCES)} pinned divergences)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
