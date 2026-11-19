#!/usr/bin/env python3
"""Accept/reject and ISA-legality gate for the AVX1 and AVX2 public intrinsics.

Every function-like intrinsic declared by `include/avxintrin.h` and
`include/avx2intrin.h` is compiled as a one-call translation unit with both
GCC 16.2 (the reference) and LCCC, under the same flags. The gate fails on:

1. **Gate hole.** LCCC accepts a call that GCC rejects. Under `-mavx` the call
   is an AVX2 intrinsic (GCC: "target specific option mismatch"), so LCCC must
   reject it too.
2. **Over-rejection.** GCC accepts a call that LCCC rejects. That is a
   correctness gap in the other direction and is also a failure.
3. **ISA leak.** A call both compilers accept, compiled by LCCC under `-mavx`,
   emits an AVX2-only instruction. An AVX-only CPU (Sandy Bridge, Ivy Bridge)
   faults on it, so the code is illegal for the requested target.

The ISA classifier is conservative in one direction only. It flags an
instruction when it is AVX2-only for every operand form; VEX.128 integer
operations, `vptest`, `vpermilps`/`vpermilpd` and `vmovdqu`/`vmovdqa` are
AVX-legal and are never flagged.

`--self-test` checks the classifier against hand-written cases first, so a
classifier regression cannot silently pass the gate.
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_REFERENCE = Path.home() / ".cache/lccc/agent-toolchains/bin/gcc-16"
HEADERS = ("include/avxintrin.h", "include/avx2intrin.h")

# AVX2-only mnemonics: illegal on an AVX-only CPU in every operand form.
# `vmaskmovps` and `vmaskmovpd` are NOT in this set: they are the AVX1
# sign-bit masked float forms (CPUID.AVX). GCC 16.2 emits them under plain
# `-mavx` and rejects only the integer `_mm256_maskload_epi32`; only the
# integer `vpmaskmov{d,q}` family is AVX2, and the prefix below covers it.
AVX2_ANY_FORM = (
    "vinserti128", "vextracti128", "vbroadcasti128", "vperm2i128",
    "vpermd", "vpermq", "vpermpd", "vpermps", "vpblendd",
    "vpsllvd", "vpsllvq", "vpsrlvd", "vpsrlvq", "vpsravd",
)
# Mnemonic prefixes that are AVX2-only in every form (vpbroadcast*, vpgather*,
# and the integer masked moves vpmaskmovd/q).
AVX2_ANY_FORM_PREFIX = ("vpbroadcast", "vpgather", "vgather", "vpmaskmov")
# Mnemonics that start with `vp` but are AVX-legal (exact names).
AVX_LEGAL_VP = frozenset({"vpermilps", "vpermilpd", "vptest", "vpclmulqdq"})
# `vbroadcastss` and `vbroadcastsd` are AVX from memory, AVX2 from a register.
VBROADCAST_REG = ("vbroadcastss", "vbroadcastsd")


def classify(line: str) -> str | None:
    """Return the offending mnemonic if `line` is an AVX2-only instruction."""
    parts = line.strip().split(None, 1)
    if not parts:
        return None
    mnemonic = parts[0]
    operands = parts[1] if len(parts) == 2 else ""
    if mnemonic in AVX2_ANY_FORM or mnemonic.startswith(AVX2_ANY_FORM_PREFIX):
        return mnemonic
    if mnemonic in VBROADCAST_REG:
        # The source is the first operand in AT&T syntax.
        src = operands.split(",", 1)[0].strip()
        return mnemonic if src.startswith("%xmm") else None
    if mnemonic == "vmovntdqa" and "%ymm" in operands:
        return mnemonic
    if mnemonic.startswith("vp") and mnemonic not in AVX_LEGAL_VP:
        # VEX.256 integer operations are AVX2; VEX.128 forms are AVX1.
        return mnemonic if "%ymm" in operands else None
    return None


def self_test() -> None:
    cases = {
        "vpand %ymm1,%ymm2,%ymm3": "vpand",
        "vpand %xmm1,%xmm2,%xmm3": None,
        "vpbroadcastd %xmm0,%ymm0": "vpbroadcastd",
        "vpbroadcastb %xmm0,%xmm0": "vpbroadcastb",
        "vbroadcastss %xmm0,%ymm0": "vbroadcastss",
        "vbroadcastss (%rdi),%ymm0": None,
        "vbroadcastsd (%rdi),%ymm0": None,
        "vbroadcastsd %xmm0,%ymm0": "vbroadcastsd",
        "vinserti128 $1,%xmm1,%ymm0,%ymm0": "vinserti128",
        "vextracti128 $0x1,%ymm0,%xmm1": "vextracti128",
        "vinsertf128 $1,%xmm1,%ymm0,%ymm0": None,
        "vextractf128 $0x1,%ymm0,%xmm1": None,
        "vpxor %ymm0,%ymm0,%ymm0": "vpxor",
        "vpxor %xmm0,%xmm0,%xmm0": None,
        "vpermilps $0x1b,%ymm0,%ymm0": None,
        "vpermilpd $0x5,%ymm0,%ymm0": None,
        "vptest %ymm0,%ymm0": None,
        "vpermd %ymm0,%ymm1,%ymm2": "vpermd",
        "vpermq $0x4e,%ymm0,%ymm1": "vpermq",
        "vpermpd $0x4e,%ymm0,%ymm1": "vpermpd",
        "vperm2i128 $0x20,%ymm1,%ymm0,%ymm0": "vperm2i128",
        "vpblendd $0xf0,%ymm1,%ymm0,%ymm0": "vpblendd",
        "vpsllvd %ymm1,%ymm0,%ymm0": "vpsllvd",
        "vpmaskmovd (%rdi),%ymm1,%ymm0": "vpmaskmovd",
        "vmovntdqa (%rdi),%ymm0": "vmovntdqa",
        "vmovntdqa (%rdi),%xmm0": None,
        "vmovdqu (%rdi),%ymm0": None,
        "vmovdqa %ymm0,%ymm1": None,
        "vxorps %ymm0,%ymm0,%ymm0": None,
        "vandps (%rdi),%ymm0,%ymm1": None,
        "vzeroupper": None,
        "vgatherdps %ymm2,(%rdi,%ymm1,4),%ymm0": "vgatherdps",
        "vaddps %ymm1,%ymm0,%ymm0": None,
        "vfmadd231ps %ymm1,%ymm0,%ymm2": None,
    }
    failures = []
    for line, want in cases.items():
        got = classify(line)
        if got != want:
            failures.append(f"  {line!r}: want {want!r}, got {got!r}")
    if failures:
        sys.exit("self-test FAILED:\n" + "\n".join(failures))
    print(f"self-test: {len(cases)} classifier cases passed")


def declared_intrinsics() -> dict[str, tuple[str, str]]:
    """Map each function-like intrinsic to (header, parameter list)."""
    pattern = re.compile(r"\b(_mm256_\w+)\s*\(([^()]*)\)\s*\{", re.S)
    found: dict[str, tuple[str, str]] = {}
    for header in HEADERS:
        text = (ROOT / header).read_text()
        text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
        text = re.sub(r"//[^\n]*", "", text)
        for match in pattern.finditer(text):
            name, params = match.group(1), match.group(2).strip()
            found.setdefault(name, (header, params))
    return found


def argument_expression(ctype: str) -> str:
    """A well-typed placeholder for one parameter of type `ctype`."""
    t = re.sub(r"\b(__restrict|restrict|const|volatile)\b", " ", ctype)
    t = re.sub(r"\s+", " ", t).strip()
    if "*" in t:
        base = t.replace("*", " ").strip()
        return "(void *)0" if base == "void" else f"({base} *)0"
    if re.match(r"__m(64|128|256)[di]?\b", t):
        return f"({t}){{0}}"
    if re.search(r"\b(double|float)\b", t):
        return "1.0"
    return "1"


def call_arguments(params: str) -> str:
    if not params or params == "void":
        return ""
    out = []
    for param in params.split(","):
        match = re.match(r"^(.*?)\s*\b\w+\s*$", param.strip())
        ctype = match.group(1) if match and match.group(1).strip() else param.strip()
        out.append(argument_expression(ctype))
    return ", ".join(out)


def compile_one(job: tuple[str, str, Path, Path, list[str]]) -> dict:
    name, params, tmp, lccc, flags = job
    src = tmp / f"{name}.c"
    src.write_text(
        "#include <immintrin.h>\n"
        f"void t(void) {{ (void){name}({call_arguments(params)}); }}\n"
    )
    result: dict = {"name": name}
    for who, compiler in (("gcc", REFERENCE), ("lccc", lccc)):
        obj = tmp / f"{name}.{who}.o"
        proc = subprocess.run(
            [str(compiler), *flags, "-c", str(src), "-o", str(obj)],
            capture_output=True, text=True, timeout=300,
        )
        result[who] = proc.returncode
        diagnostics = [ln for ln in proc.stderr.splitlines() if ": error:" in ln]
        result[who + "_err"] = (diagnostics or proc.stderr.strip().splitlines()[:1])[:1]
        if proc.returncode == 0 and who == "lccc":
            dis = subprocess.run(
                ["objdump", "-d", "--no-show-raw-insn", str(obj)],
                capture_output=True, text=True, timeout=300,
            )
            leaks = sorted({
                m for line in dis.stdout.splitlines() if "\t" in line
                for m in [classify(line.split("\t", 1)[1])] if m
            })
            result["leaks"] = leaks
    return result


REFERENCE = DEFAULT_REFERENCE


def main() -> int:
    global REFERENCE
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--lccc", type=Path, required=True)
    parser.add_argument("--reference", type=Path, default=DEFAULT_REFERENCE,
                        help="GCC 16.2 driver used as the oracle")
    parser.add_argument("--flags", default="-O2 -mavx",
                        help="flags for both compilers (default: -O2 -mavx)")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()

    self_test()
    if args.self_test:
        return 0
    if not args.reference.exists():
        print(f"error: reference compiler {args.reference} not found", file=sys.stderr)
        return 2
    REFERENCE = args.reference
    flags = args.flags.split()
    names = declared_intrinsics()
    if len(names) < 100:
        print(f"error: parsed only {len(names)} intrinsics; header format changed?",
              file=sys.stderr)
        return 2

    with tempfile.TemporaryDirectory(prefix="avx-intrinsic-isa-") as tmp:
        jobs = [(n, names[n][1], Path(tmp), args.lccc, flags) for n in sorted(names)]
        with cf.ThreadPoolExecutor(max_workers=max(1, (os.cpu_count() or 2))) as pool:
            results = list(pool.map(compile_one, jobs))

    gate_holes = [r for r in results if r["gcc"] != 0 and r["lccc"] == 0]
    over_rejects = [r for r in results if r["gcc"] == 0 and r["lccc"] != 0]
    both = [r for r in results if r["gcc"] == 0 and r["lccc"] == 0]
    leaks = [r for r in both if r.get("leaks")]
    both_reject = [r for r in results if r["gcc"] != 0 and r["lccc"] != 0]

    print(f"flags={' '.join(flags)} intrinsics={len(results)} "
          f"both_accept={len(both)} both_reject={len(both_reject)} "
          f"gate_holes={len(gate_holes)} over_rejects={len(over_rejects)} "
          f"isa_leaks={len(leaks)}")
    for label, group in (("GATE HOLE (lccc accepts, GCC rejects)", gate_holes),
                         ("OVER-REJECTION (GCC accepts, lccc rejects)", over_rejects)):
        if group:
            print(f"== {label}")
            for r in group:
                print(f"  {r['name']}: {' | '.join(r.get('lccc_err') or r.get('gcc_err') or ['?'])[:200]}")
    if leaks:
        print("== ISA LEAK (accepted by both; lccc emits AVX2-only instruction)")
        for r in leaks:
            print(f"  {r['name']}: {', '.join(r['leaks'])}")
    if args.verbose:
        for r in both:
            print(f"  ok {r['name']} lccc_mnemonics_checked")

    failed = bool(gate_holes or over_rejects or leaks)
    if failed:
        print("check_avx_intrinsic_isa: FAILED")
        return 1
    print("check_avx_intrinsic_isa: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
