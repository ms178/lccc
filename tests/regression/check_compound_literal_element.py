#!/usr/bin/env python3
"""A compound literal used as a designated aggregate element (C11 6.5.2.5p5).

WHY THIS GATE EXISTS
====================
`(T){ ... }` is an lvalue of type `T`, and in a static initializer it names
exactly the bytes the braced list `{ ... }` names.  Linux 6.18 initializes
`nodemask_t node_states[]` that way:

    #define NODE_MASK_ALL ((nodemask_t) { { [BITS_TO_LONGS(MAX_NUMNODES)-1] = ... } })
    nodemask_t node_states[NR_NODE_STATES] = { [N_POSSIBLE] = NODE_MASK_ALL, ... }

(mm/page_alloc.c).  Until this gate existed, every aggregate path in the
lowering handed that expression to the *scalar* evaluator, which can only
answer "initializer element is not constant" — so lccc could not build
mm/page_alloc.c with the user's config.  The failure was not uniform: the
relocation-aware image (`T * __percpu *` members) already unwrapped the
literal, the byte image did not, so the same source compiled or failed
depending on whether the element type happened to contain a pointer.

WHAT IS CHECKED, PER ROW
========================
1. Verdict parity with the oracle.  `gcc-16` is the reference: a row the
   oracle accepts must be accepted by lccc, and a row it rejects must be
   rejected.  (A cast around a compound literal, `(T)(T){...}`, is NOT a
   constant expression — C11 6.6p6 — and both compilers must reject it.)
2. Run-time values.  Every accepting row carries `# probe: <value>`; the probe
   function is compiled and run by BOTH compilers and the printed values must
   be identical.  This is what catches the second failure mode: an element that
   silently keeps a zero because the literal reached the scalar path.
3. Object image.  For every accepting row the relocation table
   (symbol, type, addend) of `.data`/`.data.rel.ro` must match the oracle's,
   so "accepted with the right value in the wrong slot" cannot pass.

USAGE
    GCC=gcc-16 CCC=target/fastbuild/lccc \
        tests/regression/check_compound_literal_element.py [--verbose]

EXIT STATUS
    0  every row's verdict, values and relocations match the oracle
    1  a mismatch (the failing row is named on stderr)
    2  the oracle or the compiler is missing, or is not the pinned version
"""

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CASES = Path(__file__).with_name("compound_literal_element") / "cases.tsv"

PRELUDE = """#include <stdio.h>
"""


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def version_of(cc, pinned):
    out = run([cc, "--version"])
    if out.returncode != 0:
        raise SystemExit(f"gate: {cc}: not usable ({out.stderr.strip()[:80]})")
    line = out.stdout.splitlines()[0]
    if pinned not in line:
        raise SystemExit(f"gate: {cc}: expected {pinned}, got {line}")
    return line


def parse_cases(path):
    rows = []
    for lineno, raw in enumerate(path.read_text().splitlines(), 1):
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        fields = raw.split("\t")
        if lineno == 1 and fields[0] == "id":
            continue
        if len(fields) != 3:
            raise SystemExit(f"{path}:{lineno}: expected 3 tab fields, got {len(fields)}")
        rows.append((fields[0], fields[1], fields[2]))
    return rows


def split_payload(payload):
    """`<declarations> # probe: <value>` -> (declarations, expected value)."""
    marker = "# probe:"
    if marker not in payload:
        return payload, None
    decls, _, tail = payload.rpartition(marker)
    return decls.strip(), tail.strip()


#: Relocation sections that make up the OBJECT IMAGE of a static initializer.
#: `.text` relocations are code (a compiler's own addressing choices: PC-relative
#: vs absolute, one reference vs two), so comparing them would compare codegen,
#: not the initializer.  These six are the whole answer for "did the literal
#: land in the right slot, with the right symbol".
IMAGE_RELOC_SECTIONS = (".data", ".data.rel", ".data.rel.ro", ".data.rel.local",
                        ".bss", ".rodata")


def relocation_triples(obj):
    """(section, type, symbol+addend) for every IMAGE relocation, sorted.

    The addend inside the section is not compared: two compilers may pad the
    object differently (`.data.rel.ro.local` split, static GC), and the slot a
    relocation occupies is already pinned by the run-time probe.
    """
    out = run(["readelf", "-rW", str(obj)])
    if out.returncode != 0:
        raise SystemExit(f"gate: readelf failed on {obj}: {out.stderr.strip()[:120]}")
    triples = []
    section = None
    for line in out.stdout.splitlines():
        if line.startswith("Relocation section"):
            # Relocation section '.rela.data.rel.ro' at offset 0x... contains 2 entries:
            section = line.split("'")[1]
            section = section[len(".rela") if section.startswith(".rela") else len(".rel"):]
            continue
        if section is None:
            continue
        parts = line.split()
        # "0000000000000008  0000000000000001 R_X86_64_64  0000000000000000 g + 0"
        if len(parts) < 3 or not parts[2].startswith("R_"):
            continue
        if not any(section == s or section.startswith(s + ".") for s in IMAGE_RELOC_SECTIONS):
            continue
        target = " ".join(parts[3:]) or "-"
        # The section-relative slot offset is dropped, the symbol and addend stay.
        triples.append((section, parts[2], target))
    return sorted(triples)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    ap.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    if not Path(args.compiler).exists():
        raise SystemExit(f"gate: compiler not found: {args.compiler}")
    version_of(args.gcc, "16.2")

    rows = parse_cases(CASES)
    failures = []
    accepted = 0
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        for rid, expect, payload in rows:
            decls, want = split_payload(payload)
            src = tmp / f"{rid}.c"
            body = PRELUDE + decls
            if want is not None:
                body += f"\nint main(void) {{ printf(\"%ld\\n\", (long)({want if want.startswith('probe') else 'probe()'})); return 0; }}\n"
            src.write_text(body + "\n")

            verdicts = {}
            for name, cc in (("oracle", args.gcc), ("lccc", args.compiler)):
                res = run([cc, "-std=gnu11", "-fsyntax-only", "-w", str(src)])
                # `-fsyntax-only` skips the lowering, which is where a
                # non-constant initializer is diagnosed, so the compile is the
                # verdict that matters for static initializers.
                res = run([cc, "-std=gnu11", "-c", "-w", "-o", str(tmp / f"{rid}.{name}.o"), str(src)])
                verdicts[name] = res.returncode == 0
                if not verdicts[name] and args.verbose:
                    print(f"  {rid}: {name} rejected: {res.stderr.splitlines()[:2]}")

            if verdicts["oracle"] != (expect == "accept"):
                raise SystemExit(
                    f"gate: row {rid}: the oracle disagrees with cases.tsv "
                    f"(declared {expect}, oracle {'accepts' if verdicts['oracle'] else 'rejects'})"
                )
            if verdicts["lccc"] != verdicts["oracle"]:
                failures.append(
                    f"{rid}: lccc {'accepts' if verdicts['lccc'] else 'rejects'}, "
                    f"oracle {'accepts' if verdicts['oracle'] else 'rejects'}"
                )
                continue
            if not verdicts["oracle"]:
                continue
            accepted += 1

            if want is not None:
                values = {}
                for name in ("oracle", "lccc"):
                    exe = tmp / f"{rid}.{name}.bin"
                    link = run(
                        [args.gcc if name == "oracle" else args.compiler,
                         "-w", str(src), "-o", str(exe)]
                    )
                    if link.returncode != 0:
                        failures.append(f"{rid}: {name} link failed: {link.stderr.strip()[:120]}")
                        break
                    got = run([str(exe)])
                    if got.returncode != 0:
                        failures.append(f"{rid}: {name} probe exited {got.returncode}")
                        break
                    values[name] = got.stdout.strip()
                if len(values) == 2 and values["oracle"] != values["lccc"]:
                    failures.append(
                        f"{rid}: run-time value {values['lccc']} (lccc) != "
                        f"{values['oracle']} (oracle)"
                    )

            want_relocs = relocation_triples(tmp / f"{rid}.oracle.o")
            got_relocs = relocation_triples(tmp / f"{rid}.lccc.o")
            if want_relocs != got_relocs:
                failures.append(
                    f"{rid}: relocation table differs\n    oracle: {want_relocs}\n    lccc:   {got_relocs}"
                )

    if failures:
        print("check_compound_literal_element: FAILED", file=sys.stderr)
        for f in failures:
            print(f"  {f}", file=sys.stderr)
        return 1
    print(
        f"check_compound_literal_element: PASSED ({len(rows)} rows, {accepted} accepted; "
        "verdicts, run-time values and relocation tables against GCC 16)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
