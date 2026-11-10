#!/usr/bin/env python3
"""SV-03 conditional-load sinking: generated differential correctness matrix.

Why this exists
---------------
IR unit tests prove the shape of one transform; they cannot prove that dozens
of different `cond ? *p : *q` programs still compute the same answer after the
pass runs.  This harness GENERATES those programs from a fixed seed, compiles
each translation unit with LCCC (pass ON and OFF), GCC 16.2, Clang 23.1.5 and,
when available, authenticated ICX 2026.2, at -O0/-O2/-O3, executes every binary
on the host CPU and requires byte-identical stdout.

It also quantifies the profitability rule.  `LCCC_DUMP_IR=1` writes the FINAL
IR to stderr, where a sink is an unambiguous `Select { ... ty: Ptr }`.  Three
filtered translation units are compiled with the pass ON and OFF:

  * `full`        — every generated shape;
  * `independent` — guards that do not read either arm address (must sink);
  * `arm_guard`   — guards that DO read an arm address (must NOT sink).

A vacuous census is rejected: the IR dump must be non-empty, the OFF legs must
contain zero pointer selects, the independent ON leg must contain at least one,
and the arm-guard ON leg must remain at zero.  Dynamic, runtime-derived indices
exercise GEP/Add/Sub/cast address canonicalisation; static unrolled indices
exercise zero offsets, nested offsets and struct-field chains.

`--self-test` injects wrong receipts and weakened predicates, so a checker that
always says PASS fails loudly.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import sv03_toolchain_identity as identity  # noqa: E402

SEED = 0x53565F3033  # "SV_03"
LEVELS = ("-O0", "-O2", "-O3")
COMMON_FLAGS = ("-std=c11", "-Wall", "-Wextra", "-Werror")
SELECT_PTR = re.compile(r"Select \{[^}]*\bty: Ptr\b")

TYPES = [
    ("u8", "unsigned char"),
    ("u16", "unsigned short"),
    ("u32", "unsigned int"),
    ("u64", "unsigned long long"),
    ("i8", "signed char"),
    ("i16", "short"),
    ("i32", "int"),
    ("i64", "long long"),
]

GUARDS = ("param", "flag", "one_arm", "both_arms", "chain", "mixed")
PTRS = ("direct", "gep", "struct", "chain_gep")
# Guards whose dataflow reads one of the two arm addresses.  SV-03 must
# decline these; the others must be sunk.
GUARD_READS_ARM = frozenset(("one_arm", "both_arms", "chain"))

# Dynamic shapes use a runtime-derived index from a volatile seed.  `u8`
# arrays keep the address displacement byte-scaled, so a guard spelled with
# `a[i]` and an arm spelled with `a + i` must canonicalise to the same key.
DYNAMIC_GUARDS = (
    "dyn_arm_gep",
    "dyn_arm_add",
    "dyn_chain",
    "dyn_independent",
    "dyn_mixed",
    "dyn_arm_struct",
    "dyn_chain_gep",
    "dyn_independent_struct",
    # I8 -> I32 sign-extension tail with a runtime-derived index.
    "dyn_signed_arm",
    "dyn_signed_independent",
)
ALL_GUARDS = GUARDS + DYNAMIC_GUARDS
DYNAMIC_BASE = 72
DEFAULT_CASES = DYNAMIC_BASE + len(DYNAMIC_GUARDS)


class MatrixError(RuntimeError):
    """A contract violation, reported as a hard FAIL."""


def guard_reads_arm(kind: str) -> bool:
    return (
        kind in GUARD_READS_ARM
        or kind.startswith("dyn_arm")
        or kind.startswith("dyn_chain")
        or kind == "dyn_signed_arm"
    )


def emit_case(idx: int, rng: random.Random) -> tuple[list[str], dict]:
    ty_name, cty = TYPES[idx % len(TYPES)]
    i = rng.randrange(0, 4)
    guard_kind = GUARDS[idx % len(GUARDS)]
    ptr_kind = PTRS[idx % len(PTRS)]
    name = f"case_{idx:03d}"

    decl = [
        f"static {cty} a_{name}[8];",
        f"static {cty} b_{name}[8];",
        f"static unsigned int flag_{name}[8];",
    ]
    struct = ptr_kind == "struct"
    if struct:
        decl.append(
            f"struct box_{name} {{ {cty} pad; {cty} v; {cty} tail; }};"
        )
        decl.append(f"static struct box_{name} sa_{name}[8], sb_{name}[8];")
        pexpr = f"&sa_{name}[{i}].v"
        qexpr = f"&sb_{name}[{i}].v"
    elif ptr_kind == "chain_gep":
        pexpr = f"a_{name} + {i} + 1"
        qexpr = f"b_{name} + {i} + 1"
    else:
        pexpr = f"a_{name} + {i}"
        qexpr = f"b_{name} + {i}"

    if guard_kind == "param":
        guard = "c"
    elif guard_kind == "flag":
        guard = f"(flag_{name}[{i}] & 1U)"
    elif guard_kind == "one_arm":
        guard = f"(*({pexpr}) != 0)"
    elif guard_kind == "both_arms":
        guard = f"(*({pexpr}) < *({qexpr}))"
    elif guard_kind == "chain":
        # Must read the SAME address expression as the arms.  Using
        # `a_{name} + i` here would silently test a THIRD array for the
        # struct/chain_gep shapes and mislabel an unrelated guard as an
        # arm-address guard.
        guard = f"((*({pexpr}) + 7) > 100)"
    else:
        guard = (
            f"((c ? flag_{name}[{i}] : "
            f"flag_{name}[{(i + 1) % 8}]) & 1U)"
        )

    # Both arms are always spelled with TWO loads, so the frontend builds the
    # diamond SV-03 recognises.  (void)c keeps -Wunused-parameter quiet.
    if guard_kind == "both_arms":
        expr = f"(*({pexpr}) < *({qexpr}) ? *({pexpr}) : *({qexpr}))"
    else:
        expr = f"({guard} ? *({pexpr}) : *({qexpr}))"

    decl.append(
        f"static unsigned long long {name}(int c) {{\n"
        f"    (void)c;\n"
        f"    {cty} r = {expr};\n"
        f"    unsigned long long w = (unsigned long long)(unsigned long)r;\n"
        f"    return w ^ (w << 17);\n"
        f"}}"
    )
    meta = dict(
        name=name,
        guard=guard_kind,
        ptr=ptr_kind,
        ty=ty_name,
        cty=cty,
        struct=struct,
        dynamic=False,
        call=f"{name}(c)",
    )
    return decl, meta


def emit_dynamic_case(idx: int, rng: random.Random) -> tuple[list[str], dict]:
    kind = DYNAMIC_GUARDS[(idx - DYNAMIC_BASE) % len(DYNAMIC_GUARDS)]
    # Advance the same deterministic stream as the static generator.
    rng.randrange(0, 4)
    name = f"dyn_{idx - DYNAMIC_BASE:02d}"
    signed = kind.startswith("dyn_signed")
    cty = "signed char" if signed else "unsigned char"
    struct = kind in ("dyn_arm_struct", "dyn_independent_struct")
    decl = [
        f"static {cty} a_{name}[8];",
        f"static {cty} b_{name}[8];",
        f"static unsigned int flag_{name}[8];",
    ]
    if struct:
        decl.append(
            f"struct box_{name} {{ {cty} pad; {cty} v; {cty} tail; }};"
        )
        decl.append(f"static struct box_{name} sa_{name}[8], sb_{name}[8];")

    prelude = "(void)c; (void)i; int j = (i + 1) & 7; (void)j;"
    if struct:
        pexpr = f"&sa_{name}[i].v"
        qexpr = f"&sb_{name}[i].v"
        pg = f"sa_{name}[i].v"
        qg = f"sb_{name}[i].v"
    else:
        pexpr = f"a_{name} + i"
        qexpr = f"b_{name} + i"
        pg = f"a_{name}[i]"
        qg = f"b_{name}[i]"

    if kind == "dyn_arm_gep":
        guard = f"({pg} < {qg})"
        expr = f"({guard} ? {pg} : {qg})"
    elif kind == "dyn_arm_add":
        guard = f"({pg} < {qg})"
        expr = f"({guard} ? *({pexpr}) : *({qexpr}))"
    elif kind == "dyn_chain":
        pexpr = f"a_{name} + j"
        qexpr = f"b_{name} + j"
        guard = f"((*({pexpr}) + 7) > 100)"
        expr = f"({guard} ? *({pexpr}) : *({qexpr}))"
    elif kind == "dyn_independent":
        guard = f"(flag_{name}[i] & 1U)"
        expr = f"({guard} ? *({pexpr}) : *({qexpr}))"
    elif kind == "dyn_mixed":
        guard = f"((c ? flag_{name}[i] : flag_{name}[j]) & 1U)"
        expr = f"({guard} ? *({pexpr}) : *({qexpr}))"
    elif kind == "dyn_arm_struct":
        guard = f"({pg} < {qg})"
        expr = f"({guard} ? *({pexpr}) : *({qexpr}))"
    elif kind == "dyn_chain_gep":
        # Guard uses the GEP spelling `a[j]`; arms use the Add spelling
        # `a + j`.  Both must canonicalise to ONE address key.
        pexpr = f"a_{name} + j"
        qexpr = f"b_{name} + j"
        guard = f"(a_{name}[j] < b_{name}[j])"
        expr = f"({guard} ? *({pexpr}) : *({qexpr}))"
    elif kind == "dyn_signed_arm":
        guard = f"({pg} < {qg})"
        expr = f"({guard} ? *({pexpr}) : *({qexpr}))"
    elif kind == "dyn_signed_independent":
        guard = f"(flag_{name}[i] & 1U)"
        expr = f"({guard} ? *({pexpr}) : *({qexpr}))"
    else:  # dyn_independent_struct
        guard = f"(flag_{name}[i] & 1U)"
        expr = f"({guard} ? *({pexpr}) : *({qexpr}))"

    decl.append(
        f"static unsigned long long {name}(int c, int i) {{\n"
        f"    {prelude}\n"
        f"    {cty} r = {expr};\n"
        f"    unsigned long long w = (unsigned long long)(unsigned long)r;\n"
        f"    return w ^ (w << 17);\n"
        f"}}"
    )
    meta = dict(
        name=name,
        guard=kind,
        ptr="struct" if struct else "dynamic",
        ty="i8" if signed else "u8",
        cty=cty,
        struct=struct,
        dynamic=True,
        call=f"{name}(c, i)",
    )
    return decl, meta


def build_source(
    ncases: int, guards: tuple[str, ...] | None = None
) -> tuple[str, list[dict]]:
    """One translation unit.  `guards` restricts which shapes are emitted;
    the RNG still advances over every index so a filtered TU is a strict
    subset of the full one (identical shapes, deterministic)."""
    rng = random.Random(SEED)
    decls: list[str] = []
    funcs: list[dict] = []
    for idx in range(ncases):
        if idx >= DYNAMIC_BASE:
            block, meta = emit_dynamic_case(idx, rng)
        else:
            block, meta = emit_case(idx, rng)
        if guards is not None and meta["guard"] not in guards:
            continue
        decls.extend(block)
        funcs.append(meta)

    # init() is called explicitly from main: no `__attribute__((constructor))`,
    # which not every compiler under test implements.  It runs AFTER every
    # declaration, so C's declare-before-use rule holds.
    lines = [
        "/* Generated by tests/regression/check_cond_load_sink_matrix.py.",
        " * Do not edit: deterministic seed " + hex(SEED) + ". */",
        "#include <stdio.h>",
        "",
        "/* A volatile read makes the array contents opaque at compile time.",
        " * Without it the whole matrix constant-folds and NO conditional",
        " * load reaches the pass -- a green but vacuous test. */",
        "static volatile unsigned int gseed = 0x9e3779b9U;",
        "",
        "static unsigned long long mix(unsigned long long h, unsigned long long v) {",
        "    return (h ^ v) * 0x100000001b3ULL;",
        "}",
        "",
        *decls,
        "",
        "static void init(void) {",
    ]
    for f in funcs:
        n = f["name"]
        cty = f["cty"]
        lines += [
            "    {",
            "        int j;",
            "        unsigned int s = gseed;",
            "        for (j = 0; j < 8; ++j) {",
            f"            a_{n}[j] = ({cty})(3 + j + (int)(s & 7U) - 20);",
            f"            b_{n}[j] = ({cty})(11 + j + (int)(s & 3U) - 20);",
            f"            flag_{n}[j] = 0x9e3779b9U * (unsigned)(j + 1) + (s & 0xffU);",
        ]
        if f["struct"]:
            lines += [
                f"            sa_{n}[j].pad = a_{n}[j];",
                f"            sa_{n}[j].v = a_{n}[j];",
                f"            sa_{n}[j].tail = a_{n}[j];",
                f"            sb_{n}[j].pad = b_{n}[j];",
                f"            sb_{n}[j].v = b_{n}[j];",
                f"            sb_{n}[j].tail = b_{n}[j];",
            ]
        lines += ["        }", "    }"]
    lines += [
        "}",
        "",
        "int main(int argc, char **argv) {",
        "    (void)argv;",
        "    int c = argc & 1;",
        "    gseed = (unsigned int)argc + 0x9e3779b9U;",
        "    init();",
        "    int i = (int)(gseed & 7U);",
        "    (void)i;",
        "    unsigned long long h = 0xcbf29ce484222325ULL;",
    ]
    for f in funcs:
        lines.append(f"    h = mix(h, {f['call']});")
    lines += [
        '    printf("%016llx\\n", h);',
        "    return 0;",
        "}",
        "",
    ]
    return "\n".join(lines), funcs


def version_of(path: str) -> str:
    cp = subprocess.run(
        [path, "--version"], capture_output=True, text=True, errors="replace"
    )
    if cp.returncode != 0:
        raise MatrixError(f"cannot query compiler version: {path}: {cp.stderr.strip()}")
    return (cp.stdout + cp.stderr).strip().splitlines()[0]


def require_version(name: str, path: str, pattern: str) -> str:
    text = version_of(path)
    if not re.search(pattern, text):
        raise MatrixError(f"{name} must match /{pattern}/, got: {text}")
    return text


def compiler_path(explicit: str | None, *env_names: str) -> str | None:
    if explicit:
        return explicit
    for name in env_names:
        value = os.environ.get(name)
        if value:
            return value
    return None


def resolve(path: str | None) -> str | None:
    if not path:
        return None
    if os.path.isabs(path) or os.sep in path:
        return path if Path(path).exists() else None
    return shutil.which(path) or (path if Path(path).exists() else None)


def compile_and_run(
    source: str,
    work: Path,
    oracle: str,
    level: str,
    compiler: str,
    kill: bool,
    label: str,
) -> dict:
    src = work / f"{label}.c"
    exe = work / f"{label}.bin"
    src.write_text(source)
    env = os.environ.copy()
    env.pop("LCCC_DUMP_IR", None)
    if kill:
        env["CCC_NO_COND_LOAD_SINK"] = "1"
    else:
        env.pop("CCC_NO_COND_LOAD_SINK", None)
    cmd = [compiler, level, *COMMON_FLAGS, str(src), "-o", str(exe)]
    cp = subprocess.run(
        cmd, capture_output=True, text=True, errors="replace", env=env, timeout=180
    )
    if cp.returncode != 0:
        raise MatrixError(
            f"{oracle} {level} compile failed (rc={cp.returncode}):\n"
            f"{cp.stdout}{cp.stderr}"
        )
    run = subprocess.run(
        [str(exe)], capture_output=True, env=env, timeout=30
    )
    if run.returncode != 0:
        raise MatrixError(
            f"{oracle} {level} execution failed (rc={run.returncode}): "
            f"{run.stderr.decode('utf-8', 'replace')}"
        )
    if not run.stdout:
        raise MatrixError(f"{oracle} {level} produced empty stdout")
    return dict(
        oracle=oracle,
        level=level,
        stdout_sha256=hashlib.sha256(run.stdout).hexdigest(),
        stdout_len=len(run.stdout),
        stdout=run.stdout,
    )


def compile_ir(source: str, work: Path, label: str, lccc: str, kill: bool) -> dict:
    src = work / f"ir_{label}.c"
    asm = work / f"ir_{label}.s"
    src.write_text(source)
    env = os.environ.copy()
    env["LCCC_DUMP_IR"] = "1"
    if kill:
        env["CCC_NO_COND_LOAD_SINK"] = "1"
    else:
        env.pop("CCC_NO_COND_LOAD_SINK", None)
    cmd = [lccc, "-O2", *COMMON_FLAGS, "-S", str(src), "-o", str(asm)]
    cp = subprocess.run(
        cmd, capture_output=True, text=True, errors="replace", env=env, timeout=180
    )
    if cp.returncode != 0:
        raise MatrixError(
            f"IR census {label} compile failed (rc={cp.returncode}):\n"
            f"{cp.stdout}{cp.stderr}"
        )
    ir = cp.stderr
    if not ir:
        raise MatrixError(f"IR census {label} produced no final IR on stderr")
    return dict(
        ir_bytes=len(ir.encode("utf-8", "replace")),
        pointer_selects=len(SELECT_PTR.findall(ir)),
    )


def verify(
    receipts: list[dict],
    census: dict,
    expected_pairs: int,
    require_icx: bool,
) -> None:
    if not receipts:
        raise MatrixError("no compiler pairs were executed")
    if len(receipts) != expected_pairs:
        raise MatrixError(
            f"expected {expected_pairs} compile+run pairs, got {len(receipts)}"
        )
    pairs = {(r["oracle"], r["level"]) for r in receipts}
    if len(pairs) != expected_pairs:
        raise MatrixError("duplicate or missing oracle/level pair")
    if require_icx and not any(r["oracle"] == "icx2026" for r in receipts):
        raise MatrixError("--require-icx was set but ICX 2026.2 did not run")
    expected_sha = receipts[0]["stdout_sha256"]
    for r in receipts:
        if r["stdout_sha256"] != expected_sha:
            raise MatrixError(
                f"stdout receipt mismatch: {r['oracle']} {r['level']} has "
                f"{r['stdout_sha256']}, expected {expected_sha}"
            )
        if r["stdout_len"] <= 0:
            raise MatrixError(f"{r['oracle']} {r['level']} produced empty stdout")
    required = (
        "full_off",
        "full_on",
        "independent_off",
        "independent_on",
        "arm_guard_off",
        "arm_guard_on",
    )
    for key in required:
        if key not in census:
            raise MatrixError(f"missing IR census leg: {key}")
    for key in ("full_off", "independent_off", "arm_guard_off"):
        if census[key] != 0:
            raise MatrixError(f"pass-OFF leg {key} has {census[key]} pointer selects")
    if census["independent_on"] <= 0:
        raise MatrixError("independent guards did not sink (vacuous ON census)")
    if census["arm_guard_on"] != 0:
        raise MatrixError(
            f"arm-address guards were sunk ({census['arm_guard_on']} pointer selects)"
        )
    if census["full_on"] != census["independent_on"] + census["arm_guard_on"]:
        raise MatrixError(
            f"full ON census is not the sum of its filtered legs: {census['full_on']} "
            f"!= {census['independent_on']} + {census['arm_guard_on']}"
        )
    for key, value in census.get("ir_bytes", {}).items():
        if value <= 0:
            raise MatrixError(f"vacuous IR dump for {key}")


def self_test() -> int:
    identity_failures = identity.self_test()
    if identity_failures:
        raise MatrixError(f"toolchain identity mutations accepted: {identity_failures}")
    valid_receipts = [
        dict(oracle=o, level=lv, stdout_sha256="a" * 64, stdout_len=17)
        for o in ("lccc_on", "lccc_off")
        for lv in LEVELS
    ]
    valid_census = dict(
        full_off=0,
        full_on=12,
        independent_off=0,
        independent_on=12,
        arm_guard_off=0,
        arm_guard_on=0,
        ir_bytes={k: 1000 for k in ("full_off", "full_on", "independent_off", "independent_on", "arm_guard_off", "arm_guard_on")},
    )
    verify(valid_receipts, valid_census, 6, False)
    mutations = []

    def mutate(name: str, fn) -> None:
        r = copy.deepcopy(valid_receipts)
        c = copy.deepcopy(valid_census)
        fn(r, c)
        try:
            verify(r, c, 6, False)
        except MatrixError:
            mutations.append(name)
        else:
            raise MatrixError(f"self-test mutation was NOT rejected: {name}")

    mutate("mismatched receipt", lambda r, c: r[0].__setitem__("stdout_sha256", "b" * 64))
    mutate("missing compiler pair", lambda r, c: r.pop())
    mutate("OFF leg sank a load", lambda r, c: c.__setitem__("independent_off", 1))
    mutate("vacuous ON census", lambda r, c: c.__setitem__("independent_on", 0))
    mutate("arm-address guard sank", lambda r, c: c.__setitem__("arm_guard_on", 2))
    mutate("empty IR dump", lambda r, c: c["ir_bytes"].__setitem__("full_on", 0))
    if len(mutations) != 6:
        raise MatrixError(f"expected 6 rejected mutations, got {len(mutations)}")
    print(f"PASS cond-load-sink matrix self-test ({len(mutations)} synthetic "
          f"verdict mutations rejected, toolchain identity pinned)")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cases", type=int, default=DEFAULT_CASES)
    ap.add_argument("--json", type=Path)
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--lccc", default="target/fastbuild/lccc")
    ap.add_argument("--gcc", help="GCC 16.2 binary (default $GCC16 / $CC / gcc-16)")
    ap.add_argument(
        "--clang", help="Clang 23.1.5 binary (default $CLANG23 / $CLANG / clang-23)"
    )
    ap.add_argument(
        "--icx", help="ICX 2026.2 binary (default $ICX / $LCCC_ICX_BIN / icx)"
    )
    ap.add_argument(
        "--require-icx", action="store_true", help="fail if ICX 2026.2 is absent"
    )
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    if args.cases < 1:
        print("FAIL: --cases must be positive", file=sys.stderr)
        return 1

    started = time.monotonic()
    report: dict = dict(verdict="FAIL", cases=args.cases, levels=list(LEVELS))
    try:
        lccc = resolve(args.lccc)
        if not lccc:
            raise MatrixError(f"LCCC fastbuild binary not found: {args.lccc}")
        gcc = resolve(compiler_path(args.gcc, "GCC16", "CC") or "gcc-16")
        clang = resolve(compiler_path(args.clang, "CLANG23", "CLANG") or "clang-23")
        if not gcc:
            raise MatrixError("GCC 16.2 oracle not found")
        if not clang:
            raise MatrixError("Clang 23.1.5 oracle not found")
        icx = resolve(compiler_path(args.icx, "ICX", "LCCC_ICX_BIN") or "icx")
        versions = {
            "gcc16": require_version("GCC", gcc, r"16\.2\.0"),
            "clang23": require_version("Clang", clang, r"23\.1\.5"),
        }
        oracles: list[tuple[str, str, bool]] = [
            ("lccc_on", lccc, False),
            ("lccc_off", lccc, True),
            ("gcc16", gcc, False),
            ("clang23", clang, False),
        ]
        skipped: list[str] = []
        if icx:
            versions["icx2026"] = require_version("ICX", icx, r"2026\.2")
            oracles.append(("icx2026", icx, False))
        else:
            skipped.append("icx2026")
            if args.require_icx:
                raise MatrixError("--require-icx was set but ICX 2026.2 is absent")

        full_src, shapes = build_source(args.cases)
        independent_src, _ = build_source(
            args.cases, tuple(g for g in ALL_GUARDS if not guard_reads_arm(g))
        )
        arm_src, arm_shapes = build_source(
            args.cases, tuple(g for g in ALL_GUARDS if guard_reads_arm(g))
        )
        if not arm_shapes:
            raise MatrixError("arm-guard census TU is empty")
        report.update(
            shapes=len(shapes),
            dynamic_shapes=sum(1 for s in shapes if s["dynamic"]),
            arm_guard_shapes=len(arm_shapes),
            source_sha256=hashlib.sha256(full_src.encode()).hexdigest(),
            compiler_versions=versions,
            oracles=[o[0] for o in oracles],
            skipped_oracles=skipped,
        )

        with tempfile.TemporaryDirectory(prefix="lccc-cls-matrix-") as td:
            work = Path(td)
            receipts: list[dict] = []
            for oracle, compiler, kill in oracles:
                for level in LEVELS:
                    label = f"{oracle}_{level[1:]}_{'off' if kill else 'on'}"
                    receipts.append(
                        compile_and_run(
                            full_src, work, oracle, level, compiler, kill, label
                        )
                    )
            census: dict = {}
            ir_bytes: dict = {}
            for name, source in (
                ("full", full_src),
                ("independent", independent_src),
                ("arm_guard", arm_src),
            ):
                for suffix, kill in (("off", True), ("on", False)):
                    key = f"{name}_{suffix}"
                    result = compile_ir(source, work, key, lccc, kill)
                    census[key] = result["pointer_selects"]
                    ir_bytes[key] = result["ir_bytes"]
            census["ir_bytes"] = ir_bytes

        verify(receipts, census, len(oracles) * len(LEVELS), args.require_icx)
        receipt = receipts[0]["stdout_sha256"]
        report.update(
            verdict="PASS",
            pairs=len(receipts),
            receipt=receipt,
            census=census,
            stdout_sha256_by_pair={
                f"{r['oracle']}:{r['level']}": r["stdout_sha256"] for r in receipts
            },
            elapsed_s=round(time.monotonic() - started, 3),
        )
        print(
            f"PASS cond-load-sink differential matrix ({args.cases} generated shapes, "
            f"{len(receipts)} compile+run pairs, receipt={receipt[:16]}, "
            f"oracles={','.join(o[0] for o in oracles)})"
        )
        if skipped:
            print(f"     skipped oracles: {','.join(skipped)}")
        for key in (
            "arm_guard_off",
            "arm_guard_on",
            "full_off",
            "full_on",
            "independent_off",
            "independent_on",
        ):
            print(f"     IR pointer-selects {key}: {census[key]}")
        rc = 0
    except (MatrixError, subprocess.TimeoutExpired, OSError) as exc:
        report["error"] = str(exc)
        print(f"FAIL: {exc}", file=sys.stderr)
        rc = 1
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
