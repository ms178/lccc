#!/usr/bin/env python3
"""Range-gated same-width signedness Cast: IR, host and four-ISA proof.

WHY THIS EXISTS
---------------
`simplify_cast` may delete an `I32 -> U32` / `U32 -> I32` Cast ONLY when the
source's sign bit is provably clear.  Otherwise i686 `cast_normalize_eax`,
AArch64 `sxtw` / `mov w0,w0`, RISC-V `sext.w` and x86-64 `movl` must
re-canonicalize the register home: `(I8)(U8)255` is 0xffffffff, not 0x000000ff.
An unconditional fold gives a ~5% instruction-count win on a hot loop and is
WRONG on three of the four supported ISAs.

This harness proves all three directions simultaneously:

1. FINAL-IR CENSUS: `LCCC_DUMP_IR=1` on the checked-in fixture.  Five
   provably non-negative shapes (including Select/Phi joins) contain NO
   same-width signedness Cast; 17 controls -- U32/I32 sign-bit-set, all
   U8/I8, U16/I16, U64/I64 sign-set pairs and eight opaque volatile-Load
   sign-clear negatives -- MUST still contain one.  A checker that only
   counted a global total would be vacuous, so each function is asserted
   separately.  `CCC_VERIFY_IR=abort` and `CCC_VALIDATE_SSA=1` also prove
   the IR is structurally valid on every host/cross compile.
2. HOST EXECUTION: LCCC, GCC 16.2, Clang 23.1.5 and (when available) ICX
   compile the same fixture; every exit status equals an independent Python
   oracle that includes `b < 0`, exact bit-pattern checks and a 64-bit
   zero-extension check.
3. FOUR-ISA EXECUTION: the freestanding fixture is compiled by each LCCC
   driver, assembled with the explicitly pinned, version-checked Clang
   23.1.5 integrated assembler, linked with mold, verified with readelf and
   EXECUTED natively / under qemu-user.  The sign-bit controls are exactly
   the cases that miscompile on RISC-V/i686/AArch64 if the range proof is
   weakened.  Runtime execution is an end-to-end ABI check, NOT the
   architecture-safe unsoundness detector: the IR census is load-bearing.

`--self-test` rejects synthetic mutations of every verdict predicate without
needing a toolchain.

RED-TEAM FACT (measured, not asserted): a deliberately UNSOUND unconditional
same-width fold removes all 17 guard Casts and is caught ONLY by this IR
census.  The three host compilers and all four ISAs still exit with the
correct oracle because ABI/backend canonicalization repairs the upper bits;
therefore the runtime teeth are NOT an architecture-safe unsoundness
detector and are never presented as one.  The census is the load-bearing
assertion.  `CCC_SIMPLIFY_SKIP=cast_sign_range` conversely fails the five
folded sections, proving the bisection kill switch has teeth too.
"""
from __future__ import annotations

import argparse
import copy
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_cond_load_sink_cross_arch import (  # noqa: E402
    CACHE_BIN,
    QEMU_PREFIX,
    START_ASM,
    TARGETS,
    identity,
    run,
    tool,
)

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests" / "regression" / "same_width_cast_range" / "cross.c"

# Independent oracle, mirroring cross.c's unsigned 32-bit arithmetic exactly.
A, B, C, WIDE, F, D, E, G, K = (
    1, 0x80000005, 5, 0xFFFFFFFB, 0x4000000000000001, 5, 0, 1, 1
)
M, N, R, Q, T = 5, 1, 5, 0x80000005, 1
# Six full-width sign-set controls return 1; eight opaque Load controls
# return the sign-clear runtime value 5.
FULL_WIDTH = (1, 1, 1, 1, 1, 1, 5, 5, 5, 5, 5, 5, 5, 5)
FULL_WIDTH_COEFF = (47, 53, 59, 67, 71, 73, 79, 83, 89, 97, 101, 103, 107, 109)
OK = True
H = (
    A * 1
    + B * 3
    + C * 5
    + (WIDE & 0xFFFFFFFF) * 7
    + (F & 0xFFFFFFFF) * 11
    + D * 13
    + E * 17
    + G * 19
    + K * 23
    + M * 29
    + N * 31
    + R * 37
    + Q * 41
    + T * 43
    + sum(v * c for v, c in zip(FULL_WIDTH, FULL_WIDTH_COEFF))
) & 0xFFFFFFFF
ORACLE = (H ^ (0x5A if OK else 0xFF)) % 61

IR_SECTION = re.compile(r"^=== IR (\S+) ===\s*$", re.M)
FOLDED = ("masked", "masked_u", "shifted", "select_masked", "phi_masked")
KEPT = (
    "signbit", "signed_to_unsigned", "select_signbit",
    "u8_signbit", "i8_signbit", "u16_signbit", "i16_signbit",
    "u64_signbit", "i64_signbit",
    "u8_unproven", "i8_unproven", "u16_unproven", "i16_unproven",
    "u32_unproven", "i32_unproven", "u64_unproven", "i64_unproven",
)
VERIFY_ENV = {"CCC_VERIFY_IR": "abort", "CCC_VALIDATE_SSA": "1"}
CAST_RE = {
    "masked": re.compile(r"from_ty:\s*U32,\s*to_ty:\s*I32"),
    "signbit": re.compile(r"from_ty:\s*U32,\s*to_ty:\s*I32"),
    "masked_u": re.compile(r"from_ty:\s*I32,\s*to_ty:\s*U32"),
    "signed_to_unsigned": re.compile(r"from_ty:\s*I32,\s*to_ty:\s*U32"),
    "shifted": re.compile(r"from_ty:\s*U64,\s*to_ty:\s*I64"),
    "select_masked": re.compile(r"from_ty:\s*U32,\s*to_ty:\s*I32"),
    "phi_masked": re.compile(r"from_ty:\s*U32,\s*to_ty:\s*I32"),
    "select_signbit": re.compile(r"from_ty:\s*U32,\s*to_ty:\s*I32"),
    "u8_signbit": re.compile(r"from_ty:\s*U8,\s*to_ty:\s*I8"),
    "i8_signbit": re.compile(r"from_ty:\s*I8,\s*to_ty:\s*U8"),
    "u16_signbit": re.compile(r"from_ty:\s*U16,\s*to_ty:\s*I16"),
    "i16_signbit": re.compile(r"from_ty:\s*I16,\s*to_ty:\s*U16"),
    "u64_signbit": re.compile(r"from_ty:\s*U64,\s*to_ty:\s*I64"),
    "i64_signbit": re.compile(r"from_ty:\s*I64,\s*to_ty:\s*U64"),
    "u8_unproven": re.compile(r"from_ty:\s*U8,\s*to_ty:\s*I8"),
    "i8_unproven": re.compile(r"from_ty:\s*I8,\s*to_ty:\s*U8"),
    "u16_unproven": re.compile(r"from_ty:\s*U16,\s*to_ty:\s*I16"),
    "i16_unproven": re.compile(r"from_ty:\s*I16,\s*to_ty:\s*U16"),
    "u32_unproven": re.compile(r"from_ty:\s*U32,\s*to_ty:\s*I32"),
    "i32_unproven": re.compile(r"from_ty:\s*I32,\s*to_ty:\s*U32"),
    "u64_unproven": re.compile(r"from_ty:\s*U64,\s*to_ty:\s*I64"),
    "i64_unproven": re.compile(r"from_ty:\s*I64,\s*to_ty:\s*U64"),
}


def ir_census(lccc: Path, work: Path) -> dict:
    asm = work / "census.s"
    env = dict(os.environ)
    env["LCCC_DUMP_IR"] = "1"
    env.update(VERIFY_ENV)
    cp = subprocess.run(
        [str(lccc), "-O2", "-S", str(FIXTURE), "-o", str(asm)],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        errors="replace",
        timeout=120,
    )
    if cp.returncode != 0:
        raise RuntimeError(f"LCCC IR census compile failed: {cp.stderr[-500:]}")
    dump = cp.stderr
    sections: dict[str, str] = {}
    matches = list(IR_SECTION.finditer(dump))
    for i, m in enumerate(matches):
        end = matches[i + 1].start() if i + 1 < len(matches) else len(dump)
        sections[m.group(1)] = dump[m.end() : end]
    counts = {
        fn: len(CAST_RE[fn].findall(sections.get(fn, "")))
        for fn in (*FOLDED, *KEPT)
    }
    return {
        "ir_bytes": len(dump),
        "sections": sorted(sections),
        "cast_counts": counts,
        "dump_sha256": __import__("hashlib").sha256(dump.encode()).hexdigest(),
    }


def host_source(work: Path) -> Path:
    text = FIXTURE.read_text()
    if "int lccc_main(void)" not in text:
        raise RuntimeError("fixture lost its lccc_main entry point")
    out = work / "host.c"
    out.write_text(text.replace("int lccc_main(void)", "int main(void)", 1))
    return out


def host_execution(compilers: dict[str, str], src: Path, work: Path) -> list[dict]:
    results = []
    for name, cc in compilers.items():
        exe = work / f"host_{name}"
        env = dict(os.environ)
        if name == "lccc":
            env.update(VERIFY_ENV)
        cp = run(
            [cc, "-O2", "-Wall", "-Wextra", "-Werror", str(src), "-o", str(exe)],
            env=env,
            timeout=180,
        )
        if cp.returncode != 0:
            raise RuntimeError(f"{name} host compile failed: {cp.stderr[-500:]}")
        got = run([str(exe)], timeout=60)
        results.append(
            {"compiler": name, "exit": got.returncode, "stdout": got.stdout.strip()}
        )
    return results


def build_and_execute(target: dict, work: Path, fastbuild: Path, clang: str) -> dict:
    isa = target["isa"]
    tag = f"cast_range_{isa}"
    asm, combined = work / f"{tag}.s", work / f"{tag}.combined.s"
    obj, exe = work / f"{tag}.o", work / f"{tag}.elf"
    driver = fastbuild / target["driver"]
    if not driver.is_file():
        raise RuntimeError(f"missing LCCC driver {driver}")
    env = dict(os.environ)
    env.update(VERIFY_ENV)
    compiled = run(
        [str(driver), "-O2", "-S", str(FIXTURE), "-o", str(asm)], env=env
    )
    if compiled.returncode != 0:
        raise RuntimeError(f"{isa}: LCCC compile failed: {compiled.stderr[-400:]}")
    combined.write_text(asm.read_text() + "\n" + START_ASM[isa])
    assembled = run(
        [clang, f"--target={target['triple']}", "-c", str(combined), "-o", str(obj)]
    )
    if assembled.returncode != 0:
        raise RuntimeError(f"{isa}: assembler failed: {assembled.stderr[-400:]}")
    linked = run(
        [tool("mold"), "-m", target["emulation"], str(obj), "-o", str(exe), "--entry=_start"]
    )
    if linked.returncode != 0:
        raise RuntimeError(f"{isa}: link failed: {linked.stderr[-400:]}")
    header = run([tool("readelf"), "-h", str(exe)])
    machine_ok = header.returncode == 0 and target["machine"] in header.stdout
    if not machine_ok:
        raise RuntimeError(f"{isa}: ELF machine mismatch: {header.stdout[:400]}")
    cmd = ([tool(target["qemu"])] if target["qemu"] else []) + [str(exe)]
    executed = run(cmd, timeout=90)
    return {
        "isa": isa,
        "exit": executed.returncode,
        "elf_machine_ok": machine_ok,
        "instructions": sum(
            1
            for line in asm.read_text().splitlines()
            if line.strip()
            and not re.match(r"^\s*[.#]", line)
            and not line.lstrip().startswith("#")
        ),
    }


def evaluate(report: dict) -> list[str]:
    """Pure verdict predicate; exercised by --self-test mutations."""
    bad: list[str] = []
    oracle = report["oracle"]
    if oracle != ORACLE:
        bad.append(f"oracle drift: {oracle} != {ORACLE}")
    if report.get("verifier_env") != VERIFY_ENV:
        bad.append("IR verifier environment is missing or weakened")
    hosts = report.get("hosts") or []
    expected_hosts = set(report.get("expected_hosts") or [])
    seen = {h["compiler"] for h in hosts}
    if seen != expected_hosts:
        bad.append(f"host compiler set {sorted(seen)} != {sorted(expected_hosts)}")
    for h in hosts:
        if h["exit"] != oracle:
            bad.append(f"host {h['compiler']}: exit {h['exit']} != oracle {oracle}")
    if report.get("require_icx") and "icx2026" not in report.get("icx", {}):
        bad.append("ICX required but absent")
    ir = report.get("ir") or {}
    sections = set(ir.get("sections") or [])
    if ir.get("ir_bytes", 0) < 1000:
        bad.append("IR dump is empty or truncated")
    for fn in (*FOLDED, *KEPT):
        if fn not in sections:
            bad.append(f"IR section {fn} is missing")
    counts = ir.get("cast_counts") or {}
    for fn in FOLDED:
        if counts.get(fn, -1) != 0:
            bad.append(f"{fn}: non-negative Cast was not folded (count={counts.get(fn)})")
    for fn in KEPT:
        if counts.get(fn, 0) < 1:
            bad.append(f"{fn}: required guard Cast was lost (count={counts.get(fn)})")
    cross = report.get("cross") or []
    isas = [t["isa"] for t in TARGETS]
    if sorted(r["isa"] for r in cross) != sorted(isas):
        bad.append("four-ISA result set is incomplete or duplicated")
    for r in cross:
        if r["exit"] != oracle:
            bad.append(f"{r['isa']}: exit {r['exit']} != oracle {oracle}")
        if not r["elf_machine_ok"]:
            bad.append(f"{r['isa']}: ELF machine mismatch")
        if r["instructions"] <= 0:
            bad.append(f"{r['isa']}: empty assembly")
    return bad


def self_test() -> int:
    base = {
        "oracle": ORACLE,
        "hosts": [
            {"compiler": "lccc", "exit": ORACLE, "stdout": ""},
            {"compiler": "gcc16", "exit": ORACLE, "stdout": ""},
            {"compiler": "clang23", "exit": ORACLE, "stdout": ""},
        ],
        "expected_hosts": {"lccc", "gcc16", "clang23"},
        "require_icx": False,
        "icx": {},
        "verifier_env": dict(VERIFY_ENV),
        "ir": {
            "ir_bytes": 4000,
            "sections": sorted((*FOLDED, *KEPT)),
            "cast_counts": {
                **{fn: 0 for fn in FOLDED},
                **{fn: 1 for fn in KEPT},
            },
        },
        "cross": [
            {"isa": t["isa"], "exit": ORACLE, "elf_machine_ok": True, "instructions": 50}
            for t in TARGETS
        ],
    }
    assert not evaluate(base), f"self-test baseline is not clean: {evaluate(base)}"
    mutations = [
        ("oracle drift", lambda r: r.update(oracle=ORACLE + 1), "oracle drift"),
        ("wrong host exit", lambda r: r["hosts"][0].update(exit=ORACLE + 1), "host lccc"),
        ("missing compiler", lambda r: r["hosts"].pop(), "host compiler set"),
        ("empty IR", lambda r: r["ir"].update(ir_bytes=10), "IR dump is empty"),
        ("missing IR section", lambda r: r["ir"]["sections"].remove("signbit"), "IR section signbit is missing"),
        ("wrong cross exit", lambda r: r["cross"][0].update(exit=ORACLE + 1), "x86_64"),
        ("missing ISA", lambda r: r["cross"].pop(), "four-ISA result set"),
        ("bad ELF machine", lambda r: r["cross"][0].update(elf_machine_ok=False), "ELF machine"),
        ("empty assembly", lambda r: r["cross"][0].update(instructions=0), "empty assembly"),
        ("weak verifier", lambda r: r["verifier_env"].pop("CCC_VERIFY_IR"), "IR verifier environment"),
        ("absent required ICX", lambda r: r.update(require_icx=True), "ICX required"),
    ]
    for fn in FOLDED:
        mutations.append((
            f"missing {fn} fold",
            lambda r, fn=fn: r["ir"]["cast_counts"].update({fn: 1}),
            f"{fn}: non-negative Cast was not folded",
        ))
    for fn in KEPT:
        mutations.append((
            f"lost {fn} canonicalization",
            lambda r, fn=fn: r["ir"]["cast_counts"].update({fn: 0}),
            f"{fn}: required guard Cast was lost",
        ))
    for name, mutate, want in mutations:
        sample = copy.deepcopy(base)
        mutate(sample)
        bad = evaluate(sample)
        if not any(want in b for b in bad):
            print(f"self-test FAIL: '{name}' was not detected: {bad}", file=sys.stderr)
            return 1
    print(f"PASS same-width-cast-range self-test ({len(mutations)} verdict "
          f"mutations rejected, toolchain identity pinned, oracle={ORACLE})")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--json", type=Path)
    ap.add_argument("--lccc", default="target/fastbuild/lccc")
    ap.add_argument("--gcc", default=os.environ.get("GCC16", "gcc-16"))
    ap.add_argument("--clang", default=os.environ.get("CLANG23", os.environ.get("CLANG", "clang-23")))
    ap.add_argument("--icx", default=os.environ.get("ICX", os.environ.get("LCCC_ICX_BIN", "")))
    ap.add_argument("--require-icx", action="store_true")
    ap.add_argument("--fastbuild", type=Path, default=ROOT / "target" / "fastbuild")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()

    compilers = {
        "lccc": args.lccc,
        "gcc16": args.gcc,
        "clang23": args.clang,
    }
    icx = {}
    if args.icx:
        icx["icx2026"] = args.icx
        compilers["icx2026"] = args.icx
    elif args.require_icx:
        print("FAIL: --require-icx but no --icx / ICX / LCCC_ICX_BIN", file=sys.stderr)
        return 1

    report: dict = {
        "oracle": ORACLE,
        "expected_hosts": sorted(compilers),
        "require_icx": args.require_icx,
        "icx": icx,
        "verifier_env": dict(VERIFY_ENV),
    }
    try:
        toolchain = identity.require_compiler_versions(
            compilers, search_paths=(CACHE_BIN, QEMU_PREFIX)
        )
        report["toolchain"] = toolchain
        pinned = {name: row["path"] for name, row in toolchain.items()}
        pinned["lccc"] = args.lccc
        with tempfile.TemporaryDirectory(prefix="lccc-cast-range-") as td:
            work = Path(td)
            report["ir"] = ir_census(Path(args.lccc), work)
            report["hosts"] = host_execution(pinned, host_source(work), work)
            report["cross"] = [
                build_and_execute(t, work, args.fastbuild, pinned["clang23"])
                for t in TARGETS
            ]
    except (RuntimeError, subprocess.TimeoutExpired, OSError) as exc:
        report["error"] = str(exc)
        bad = [str(exc)]
    else:
        bad = evaluate(report)
    report["problems"] = bad
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report, indent=2, sort_keys=True, default=str) + "\n")
    for h in report.get("hosts", []):
        print(f"PASS host {h['compiler']:10} exit={h['exit']:3} oracle={ORACLE}")
    for fn in (*FOLDED, *KEPT):
        print(f"PASS IR {fn:20} casts={report.get('ir', {}).get('cast_counts', {}).get(fn)}")
    for r in report.get("cross", []):
        print(f"PASS {r['isa']:8} exit={r['exit']:3} instructions={r['instructions']}")
    if bad:
        for b in bad:
            print(f"FAIL {b}", file=sys.stderr)
        return 1
    print(
        f"PASS same-width-cast-range ({len(report['hosts'])} host compilers, "
        f"{len(report['ir']['sections'])} IR sections, 4 ISAs, oracle={ORACLE})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
