#!/usr/bin/env python3
"""boot_ab_size.py — paired A/B of two LCCC binaries over the x86 boot corpus.

`boot_size_oracle.sh` compares LCCC against a *reference compiler*.  This
tool compares LCCC against LCCC: two binaries (or one binary under two
environments) compile the identical `arch/x86/boot` command lines from
`scripts/boot_flags.sh`, and the report is per-object executable-byte
deltas plus the aggregate and the resulting 32 KiB-gate headroom.

That is the only way to attribute a code-size movement to a codegen change:
the oracle comparison always also contains the (large, unrelated) gap to the
reference, so a 300-byte win is invisible in it and a 300-byte regression
looks like noise.

Scope, stated so the absolute numbers are not misread: this compiles the
C members of `LCCC_BOOT_C_FILES` only, not the four `.S` members, so its
TOTAL is not comparable to `boot_size_oracle.sh`'s (which counts all 23
objects).  Both arms always see the identical file set, so the DELTA — the
only thing this tool is for — is exact.  Self-A/B (the same binary in both
arms) returns +0 on every object.

Both arms run in the same process, back to back, on the same tree, so a
delta cannot come from a stale generated header.

Usage:
    scripts/boot_ab_size.py --a target/fastbuild/lccc --b target-ab/fastbuild/lccc
    scripts/boot_ab_size.py --a ... --b ... --b-env CCC_NO_I686_ACCUM_NOHOME=1
    scripts/boot_ab_size.py --a ... --b ... --opt -O2
"""
from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def boot_flags(kernel: Path) -> tuple[list[str], list[str]]:
    out = subprocess.run(
        ["bash", "-c", f'. {HERE}/boot_flags.sh; printf "%s\\n" "$LCCC_BOOT_CFLAGS"; printf "%s\\n" "$LCCC_BOOT_CPPFLAGS"; printf "%s\\n" "${{LCCC_BOOT_C_FILES[*]}}"'],
        cwd=kernel, capture_output=True, text=True, check=True,
    ).stdout.splitlines()
    return out[0].split(), out[1].split(), out[2].split()


def code_bytes(obj: Path) -> int:
    """Executable bytes = the sections the boot linker script places as code.

    Mirrors elf_sections.sh's lccc_elf_code_bytes: .text*, .bstext, .header,
    .entrytext, .inittext. `size -A` is used rather than readelf so the
    parser does not have to re-implement ELF.
    """
    out = subprocess.run(["size", "-A", str(obj)], capture_output=True, text=True).stdout
    total = 0
    for line in out.splitlines():
        parts = line.split()
        if len(parts) < 2:
            continue
        name = parts[0]
        if name.startswith(
            (".text", ".bstext", ".header", ".entrytext", ".inittext", ".pecompat", ".videocards")
        ):
            try:
                total += int(parts[1])
            except ValueError:
                pass
    return total


def compile_arm(cc: str, cflags: list[str], cppflags: list[str], files: list[str],
                kernel: Path, outdir: Path, env: dict[str, str]) -> dict[str, int]:
    outdir.mkdir(parents=True, exist_ok=True)
    e = dict(os.environ)
    e.update(env)
    res: dict[str, int] = {}
    for f in files:
        obj = outdir / f"{f}.o"
        cmd = [cc, *cppflags, *cflags, "-DSVGA_MODE=NORMAL_VGA", "-c",
               f"arch/x86/boot/{f}.c", "-o", str(obj)]
        p = subprocess.run(cmd, cwd=kernel, capture_output=True, text=True, env=e)
        if p.returncode != 0:
            print(f"  {f}: {cc} failed: {p.stderr.strip()[:200]}", file=sys.stderr)
            continue
        res[f] = code_bytes(obj)
    return res


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--kernel", default=os.environ.get("KERNEL_DIR", "/opt/kwork/linux-6.18.52"))
    ap.add_argument("--a", required=True, help="baseline lccc binary")
    ap.add_argument("--b", required=True, help="candidate lccc binary")
    ap.add_argument("--a-env", default="", help="KEY=V[,KEY=V] for arm A")
    ap.add_argument("--b-env", default="", help="KEY=V[,KEY=V] for arm B")
    ap.add_argument("--work", default="/tmp/boot-ab")
    a = ap.parse_args()

    def envmap(s: str) -> dict[str, str]:
        d = {}
        for kv in filter(None, s.split(",")):
            k, _, v = kv.partition("=")
            d[k] = v
        return d

    kernel = Path(a.kernel)
    # Resolve the compiler paths before cwd moves into the kernel tree.
    arm_a = str(Path(a.a).expanduser().resolve())
    arm_b = str(Path(a.b).expanduser().resolve())
    for p_ in (arm_a, arm_b):
        if not Path(p_).is_file():
            print(f"compiler not found: {p_}", file=sys.stderr)
            return 2
    cflags, cppflags, files = boot_flags(kernel)
    ra = compile_arm(arm_a, cflags, cppflags, files, kernel, Path(a.work) / "a", envmap(a.a_env))
    rb = compile_arm(arm_b, cflags, cppflags, files, kernel, Path(a.work) / "b", envmap(a.b_env))
    common = sorted(set(ra) & set(rb))
    if not common:
        print("no object compiled by both arms", file=sys.stderr)
        return 2

    ta = sum(ra[k] for k in common)
    tb = sum(rb[k] for k in common)
    print(f"corpus: {len(common)} objects from {kernel}/arch/x86/boot")
    print(f"A: {arm_a} {a.a_env}")
    print(f"B: {arm_b} {a.b_env}")
    print(f"\n{'object':<24}{'A':>8}{'B':>8}{'delta':>8}")
    for k in sorted(common, key=lambda k: rb[k] - ra[k]):
        d = rb[k] - ra[k]
        mark = "  <-- better" if d < 0 else ("  <-- WORSE" if d > 0 else "")
        print(f"{k:<24}{ra[k]:>8}{rb[k]:>8}{d:>+8}{mark}")
    print(f"{'TOTAL':<24}{ta:>8}{tb:>8}{tb - ta:>+8}")
    pct = 100.0 * (tb - ta) / max(ta, 1)
    print(f"\naggregate: {tb - ta:+d} bytes ({pct:+.2f}%)")
    # The gate is on the LINKED image, not the object sum, but the sum is the
    # right early signal; report the implied headroom either way.
    print(f"implied setup headroom change: {-(tb - ta):+d} bytes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
