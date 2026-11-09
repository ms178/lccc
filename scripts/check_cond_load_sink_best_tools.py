#!/usr/bin/env python3
"""Best-tool evidence for SV-03 + the range-gated signedness Cast fold.

Tools actually used (all pinned/verified by the CI lane):
  * GDB 18.1       -- PROT_NONE untaken-pointer proof, exact return values
  * objdump 2.47   -- static instruction census of four load widths
  * readelf 2.47   -- ELF section, machine and .eh_frame FDE-table structural
                      proof (objtool stand-in for user-space objects)
  * Callgrind      -- dynamic instruction refs on the host CPU (perf stand-in)
  * GCC 16.2 / Clang 23.1.5 / ICX 2026.2 (optional) -- differential oracles
  * QEMU 11.1.3    -- used by the companion four-ISA execution gate

`objtool`, `pahole` and `perf` are NOT installed in this sandbox or on the
hosted runner.  Rather than fake them, this gate performs the equivalent
structural checks with readelf/objdump/Callgrind and records the absence in
the JSON evidence.  No claim is made that those three tools ran.

The fold benchmark is NON-VACUOUS: an IR census proves `select_masked` has a
real U32->I32 Cast with `CCC_SIMPLIFY_SKIP=cast_sign_range` (OFF) and none
with the fold ON; Callgrind then measures the dynamic instruction delta.
A volatile-reload control, where the Cast source is an opaque Load, must
measure NO per-iteration change ON/OFF (<= 1000 refs of layout noise),
proving the proof refuses what it cannot prove.  The foldable shape must
save >= 90,000 refs (one move x 100,000 iterations).

`--self-test` rejects synthetic mutations of every verdict predicate without
needing any toolchain.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

import sv03_toolchain_identity as identity

ROOT = Path(__file__).resolve().parents[1]
CACHE_BIN = Path(
    os.environ.get("LCCC_AGENT_CACHE_PREFIX", str(Path.home() / ".cache/lccc/agent-toolchains"))
) / "bin"

NARROW = r'''
__attribute__((noinline)) unsigned char pick8(const unsigned char *p,
                                               const unsigned char *q, int c) {
    return c ? p[0] : q[0];
}
__attribute__((noinline)) short pick16(const short *p, const short *q, int c) {
    return c ? p[0] : q[0];
}
__attribute__((noinline)) int pick32(const int *p, const int *q, int c) {
    return c ? p[0] : q[0];
}
__attribute__((noinline)) float pickf(const float *p, const float *q, int c) {
    return c ? p[0] : q[0];
}
'''

PROT_NONE = r'''
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>
__attribute__((noinline)) unsigned char pick8(const unsigned char *p,
                                               const unsigned char *q, int c) {
    return c ? p[0] : q[0];
}
int main(void) {
    long ps = sysconf(_SC_PAGESIZE);
    unsigned char *good = mmap(0, (size_t)ps, PROT_READ | PROT_WRITE,
                               MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    unsigned char *bad = mmap(0, (size_t)ps, PROT_NONE,
                              MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (good == MAP_FAILED || bad == MAP_FAILED) return 2;
    good[0] = 0x5a;
    printf("%02x %02x\n", pick8(good, bad, 1), pick8(bad, good, 0));
    return 0;
}
'''

# The frontend omits its own Cast for a plain mask but emits a REAL
# U32->I32 Cast on the ternary join; the range proof removes exactly that.
BENCH_FOLD = r'''
#include <stdio.h>
static volatile unsigned int g_seed = 1;
static unsigned char A[64], B[64];
__attribute__((noinline)) unsigned char pick8(const unsigned char *p,
                                               const unsigned char *q, int c) {
    return c ? p[0] : q[0];
}
__attribute__((noinline)) static int select_masked(unsigned int x, int c) {
    unsigned int u = c ? (x & 0x7fffffffU) : (x & 1U);
    return (int)u;
}
int main(void) {
    int i;
    unsigned long long h = 0;
    for (i = 0; i < 64; ++i) {
        A[i] = (unsigned char)(i * 3 + 1);
        B[i] = (unsigned char)(i * 5 + 7);
    }
    for (i = 0; i < 100000; ++i) {
        unsigned int s = g_seed + (unsigned)i;
        int c = select_masked(s, (int)(s & 1U));
        h += pick8(A, B, c);
    }
    printf("%016llx\n", h);
    return 0;
}
'''

# Control: the Cast source is a volatile Load, which the sign-bit proof
# CANNOT prove.  ON and OFF must be byte-identical in dynamic cost.
BENCH_VOLATILE = r'''
#include <stdio.h>
static volatile unsigned int g_c = 1;
static unsigned char A[64], B[64];
__attribute__((noinline)) unsigned char pick8(const unsigned char *p,
                                               const unsigned char *q, int c) {
    return c ? p[0] : q[0];
}
int main(void) {
    int i;
    unsigned long long h = 0;
    for (i = 0; i < 64; ++i) {
        A[i] = (unsigned char)(i * 3 + 1);
        B[i] = (unsigned char)(i * 5 + 7);
    }
    for (i = 0; i < 100000; ++i) {
        g_c = (g_c + (unsigned)i) & 1U;
        int c = (int)g_c;
        h += pick8(A, B, c);
    }
    printf("%016llx\n", h);
    return 0;
}
'''

IR_SECTION = re.compile(r"^=== IR (\S+) ===\s*$", re.M)
JOIN_CAST = re.compile(r"from_ty:\s*U32,\s*to_ty:\s*I32")


def discover_tool(name: str) -> str | None:
    """One discovery rule for both the report and the commands that run it.

    PATH is searched first, then the pinned cache binary, and only an actually
    executable file counts.  Using this in preflight prevents a stale file in
    the cache from being reported as available while the command fails later.
    """
    found = shutil.which(name)
    if found:
        return found
    candidate = CACHE_BIN / name
    if candidate.is_file() and os.access(candidate, os.X_OK):
        return str(candidate)
    return None


def tool(name: str) -> str:
    found = discover_tool(name)
    if found is None:
        raise RuntimeError(f"required tool {name!r} is unavailable")
    return found


def run(cmd: list[str], *, env: dict[str, str] | None = None, timeout: int = 300):
    return subprocess.run(
        cmd, cwd=ROOT, env=env or os.environ.copy(), capture_output=True,
        text=True, errors="replace", timeout=timeout,
    )


def compile_one(cc: str, src: Path, obj: Path, *, extra_env: dict[str, str] | None = None,
                flags: tuple[str, ...] = ("-O2",)) -> None:
    env = os.environ.copy()
    env.update(extra_env or {})
    cp = run([cc, *flags, "-c", str(src), "-o", str(obj)], env=env)
    if cp.returncode != 0:
        raise RuntimeError(f"{cc} compile failed: {cp.stderr[-600:]}")


def objdump_instructions(obj: Path, fn: str) -> tuple[int, list[str]]:
    cp = run([tool("objdump"), "-d", "--no-show-raw-insn", f"--disassemble={fn}", str(obj)])
    if cp.returncode != 0:
        raise RuntimeError(f"objdump {fn}: {cp.stderr}")
    lines = [ln.strip() for ln in cp.stdout.splitlines()
             if re.match(r"^\s*[0-9a-f]+:\s+\S", ln)]
    return len(lines), [re.sub(r"^\s*[0-9a-f]+:\s+", "", ln) for ln in lines]


def callgrind_refs(exe: Path, work: Path, label: str) -> int:
    out = work / f"{label}.cg"
    cp = run([tool("valgrind"), "--tool=callgrind",
              f"--callgrind-out-file={out}", str(exe)], timeout=300)
    if cp.returncode != 0:
        raise RuntimeError(f"callgrind {label}: {cp.stderr[-600:]}")
    m = re.search(r"^summary:\s+(\d+)", out.read_text(), re.M)
    if not m:
        raise RuntimeError(f"callgrind {label}: no summary in {out}")
    return int(m.group(1))


def ir_join_census(lccc: str, src: Path, work: Path) -> dict[str, int]:
    counts = {}
    for state, extra in (("on", {}), ("off", {"CCC_SIMPLIFY_SKIP": "cast_sign_range"})):
        env = os.environ.copy()
        env["LCCC_DUMP_IR"] = "1"
        env.update(extra)
        asm = work / f"fold_{state}.s"
        cp = run([lccc, "-O2", "-S", str(src), "-o", str(asm)], env=env)
        if cp.returncode != 0:
            raise RuntimeError(f"IR census {state}: {cp.stderr[-600:]}")
        dump = cp.stderr
        matches = list(IR_SECTION.finditer(dump))
        section = ""
        for i, m in enumerate(matches):
            if m.group(1) == "select_masked":
                end = matches[i + 1].start() if i + 1 < len(matches) else len(dump)
                section = dump[m.end():end]
                break
        if not section:
            raise RuntimeError(f"IR census {state}: no select_masked section")
        counts[state] = len(JOIN_CAST.findall(section))
    return counts


def gdb_prot_none(lccc: str, work: Path) -> dict:
    src = work / "prot.c"
    src.write_text(PROT_NONE)
    results = {}
    for state, extra in (("on", {}), ("off", {"CCC_NO_COND_LOAD_SINK": "1"})):
        env = os.environ.copy()
        env.update(extra)
        exe = work / f"prot_{state}"
        cp = run([lccc, "-O2", "-g", "-Wall", "-Wextra", "-Werror",
                  str(src), "-o", str(exe)], env=env)
        if cp.returncode != 0:
            raise RuntimeError(f"prot {state} compile: {cp.stderr[-600:]}")
        script = work / f"gdb_{state}.cmd"
        script.write_text(
            "set pagination off\nset confirm off\nbreak pick8\nrun\n"
            "finish\nprintf \"GDB_RET=%02x\\n\", $eax\ncontinue\n"
            "finish\nprintf \"GDB_RET=%02x\\n\", $eax\ncontinue\n"
        )
        cp = run([tool("gdb"), "-q", "-batch", "-x", str(script), str(exe)], timeout=180)
        if cp.returncode != 0:
            raise RuntimeError(f"gdb {state}: {cp.stderr[-600:]}")
        no_fault = "SIGSEGV" not in cp.stdout and "SIGSEGV" not in cp.stderr
        rets = re.findall(r"GDB_RET=([0-9a-f]+)", cp.stdout)
        results[state] = {
            "no_sigsegv": no_fault,
            "returns": rets,
            "stdout_sha256": hashlib.sha256(cp.stdout.encode()).hexdigest(),
        }
    return results


def structural_readelf(exe: Path) -> dict:
    cp = run([tool("readelf"), "-S", str(exe)])
    if cp.returncode != 0:
        raise RuntimeError(f"readelf -S: {cp.stderr}")
    sections = set(re.findall(r"\[\s*\d+\]\s+(\S+)", cp.stdout))
    # `-wf` decodes .eh_frame_hdr's binary search table.  An object with no
    # FDE entries cannot unwind through the calls the GDB canary exercises;
    # this is the user-space equivalent of objtool's ORC table sanity check.
    frames = run([tool("readelf"), "-wf", str(exe)])
    if frames.returncode != 0:
        raise RuntimeError(f"readelf -wf: {frames.stderr}")
    fde_count = len(re.findall(r"fde=\[", frames.stdout))
    return {
        "has_text": ".text" in sections,
        "has_eh_frame": ".eh_frame" in sections,
        "fde_count": fde_count,
    }


def evaluate(report: dict) -> list[str]:
    bad: list[str] = []
    tools = report.get("tools") or {}
    for required in ("gdb", "objdump", "readelf", "valgrind"):
        if not tools.get(required):
            bad.append(f"required tool {required} missing")
    static = report.get("objdump") or {}
    for fn in ("pick8", "pick16"):
        on = static.get(f"lccc_on:{fn}")
        off = static.get(f"lccc_off:{fn}")
        gcc = static.get(f"gcc16:{fn}")
        clang = static.get(f"clang23:{fn}")
        if on != 4 or gcc != 4 or clang != 4:
            bad.append(f"{fn}: LCCC/GCC/Clang not 4 instructions ({on}/{gcc}/{clang})")
        if not isinstance(off, int) or off <= on:
            bad.append(f"{fn}: SV-03 OFF is not branchier ({off} <= {on})")
    gdb = report.get("gdb") or {}
    for state in ("on", "off"):
        g = gdb.get(state) or {}
        if not g.get("no_sigsegv"):
            bad.append(f"gdb {state}: SIGSEGV (speculated load)")
        if g.get("returns") != ["5a", "5a"]:
            bad.append(f"gdb {state}: unexpected returns {g.get('returns')}")
    ir = report.get("ir_join_census") or {}
    if ir.get("on") != 0 or ir.get("off") != 1:
        bad.append(f"IR census is vacuous: on={ir.get('on')} off={ir.get('off')}")
    cg = report.get("callgrind") or {}
    checksums = cg.get("checksums") or {}
    if len(set(checksums.values())) != 1 or not checksums:
        bad.append(f"callgrind checksums disagree: {checksums}")
    refs = cg.get("refs") or {}
    fold_delta = refs.get("fold_off", 0) - refs.get("fold_on", 0)
    # One reg-reg move per iteration over 100,000 iterations; require a
    # per-iteration win, not a startup/layout artifact.
    if fold_delta < 90_000:
        bad.append(f"foldable benchmark: ON saved only {fold_delta} refs "
                   "(expected >= 90,000, i.e. one instruction/iteration)")
    volatile_delta = abs(refs.get("volatile_on", 0) - refs.get("volatile_off", 0))
    # The proof refuses the volatile Load source.  A few refs of code-layout
    # noise are allowed; a per-iteration change (>= 1000) is not.
    if volatile_delta > 1000:
        bad.append(f"volatile control changed by {volatile_delta} refs "
                   "when the proof was disabled (expected < 1000)")
    struct = report.get("structural") or {}
    if (not struct.get("has_text") or not struct.get("has_eh_frame")
            or struct.get("fde_count", 0) <= 0):
        bad.append("ELF .text/.eh_frame/FDE structural check failed")
    if report.get("require_icx") and "icx2026" not in (report.get("compilers") or {}):
        bad.append("ICX required but absent")
    if report.get("error"):
        bad.append(str(report["error"]))
    return bad


def report_evidence(report: dict) -> int:
    """Print evidence only when the whole instrument chain succeeded.

    A preflight failure leaves the measured sections absent.  The previous
    unconditional `report["objdump"]` access raised KeyError before the real
    problems could be printed, so a red gate produced no actionable diagnosis.
    This function is total over an incomplete report and is exercised directly
    by `--self-test`.
    """
    problems = report.get("problems") or []
    if report.get("verdict") != "PASS" or problems:
        cause = report.get("error") or "incomplete evidence"
        print(f"FAIL best-tool evidence: {cause}", file=sys.stderr)
        missing = [k for k, v in (report.get("tools") or {}).items() if not v]
        print(f"NOTE optional tools unavailable (not faked): {missing}", file=sys.stderr)
        for problem in problems:
            print(f"FAIL {problem}", file=sys.stderr)
        return 1

    static = report.get("objdump") or {}
    for fn in ("pick8", "pick16"):
        print(f"PASS objdump {fn}: on={static.get(f'lccc_on:{fn}')} "
              f"off={static.get(f'lccc_off:{fn}')} "
              f"gcc16={static.get(f'gcc16:{fn}')} "
              f"clang23={static.get(f'clang23:{fn}')}")
    gdb = report.get("gdb") or {}
    for state in ("on", "off"):
        g = gdb.get(state) or {}
        print(f"PASS gdb {state}: no_sigsegv={g.get('no_sigsegv')} returns={g.get('returns')}")
    print(f"PASS IR join census: {report.get('ir_join_census')}")
    print(f"PASS Callgrind refs: {(report.get('callgrind') or {}).get('refs')}")
    print(f"PASS ELF structure: {report.get('structural')}")
    missing = [k for k, v in (report.get("tools") or {}).items() if not v]
    print(f"NOTE optional tools unavailable (not faked): {missing}")
    icx_suffix = "/ICX" if "icx2026" in (report.get("compilers") or {}) else ""
    print("PASS best-tool evidence (GDB/objdump/readelf/Callgrind, GCC16/Clang23"
          + icx_suffix + ")")
    return 0


def self_test() -> int:
    identity_failures = identity.self_test()
    if identity_failures:
        for problem in identity_failures:
            print(f"self-test FAIL: toolchain identity: {problem}", file=sys.stderr)
        return 1
    base = {
        "tools": {"gdb": "gdb", "objdump": "objdump", "readelf": "readelf",
                  "valgrind": "valgrind", "objtool": None, "pahole": None, "perf": None},
        "objdump": {"lccc_on:pick8": 4, "lccc_off:pick8": 7, "gcc16:pick8": 4,
                    "clang23:pick8": 4, "lccc_on:pick16": 4, "lccc_off:pick16": 7,
                    "gcc16:pick16": 4, "clang23:pick16": 4},
        "gdb": {"on": {"no_sigsegv": True, "returns": ["5a", "5a"]},
                "off": {"no_sigsegv": True, "returns": ["5a", "5a"]}},
        "ir_join_census": {"on": 0, "off": 1},
        "callgrind": {"checksums": {"a": "X", "b": "X"},
                      "refs": {"fold_on": 2_640_446, "fold_off": 2_740_432,
                               "volatile_on": 2_040_448,
                               "volatile_off": 2_040_434}},
        "structural": {"has_text": True, "has_eh_frame": True, "fde_count": 1},
        "compilers": {"lccc": "lccc", "gcc16": "gcc", "clang23": "clang"},
        "require_icx": False,
    }
    assert not evaluate(base), f"self-test baseline not clean: {evaluate(base)}"
    import copy
    mutations = [
        ("wrong static count", lambda r: r["objdump"].update(**{"lccc_on:pick8": 5}), "not 4 instructions"),
        ("missing transform", lambda r: r["objdump"].update(**{"lccc_off:pick8": 4}), "not branchier"),
        ("GDB fault", lambda r: r["gdb"]["on"].update(no_sigsegv=False), "SIGSEGV"),
        ("wrong GDB return", lambda r: r["gdb"]["off"].update(returns=["5b", "5a"]), "unexpected returns"),
        ("vacuous IR census", lambda r: r["ir_join_census"].update(off=0), "vacuous"),
        ("fold not cheaper", lambda r: r["callgrind"]["refs"].update(fold_on=2_740_432), "saved only"),
        ("fold delta too small", lambda r: r["callgrind"]["refs"].update(fold_off=2_640_546), "saved only"),
        ("volatile control drift", lambda r: r["callgrind"]["refs"].update(volatile_off=2_140_434), "volatile control changed"),
        ("checksum drift", lambda r: r["callgrind"]["checksums"].update(b="Y"), "checksums disagree"),
        ("missing GDB", lambda r: r["tools"].update(gdb=None), "required tool gdb missing"),
        ("missing Valgrind", lambda r: r["tools"].update(valgrind=None),
         "required tool valgrind missing"),
        ("missing FDE table", lambda r: r["structural"].update(fde_count=0), "FDE structural check"),
        ("absent required ICX", lambda r: r.update(require_icx=True), "ICX required"),
    ]
    for name, mutate, want in mutations:
        sample = copy.deepcopy(base)
        mutate(sample)
        bad = evaluate(sample)
        if not any(want in b for b in bad):
            print(f"self-test FAIL: '{name}' not detected: {bad}", file=sys.stderr)
            return 1

    # Regression: a missing Valgrind used to produce an empty AssertionError,
    # then `report["objdump"]` raised KeyError while printing unearned PASS
    # lines.  A complete report still prints identically; an incomplete one
    # must return 1 with the named cause, never raise.
    import contextlib
    import io
    good = copy.deepcopy(base)
    good["verdict"], good["problems"] = "PASS", []
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        good_rc = report_evidence(good)
    if good_rc != 0 or "PASS best-tool evidence" not in out.getvalue():
        print("self-test FAIL: complete report was not accepted", file=sys.stderr)
        return 1
    broken = copy.deepcopy(good)
    broken["tools"]["valgrind"] = None
    broken["verdict"] = "FAIL"
    broken["error"] = "required tool 'valgrind' is unavailable"
    broken.pop("objdump")
    broken["problems"] = evaluate(broken)
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        bad_rc = report_evidence(broken)
    if bad_rc != 1 or broken["error"] not in err.getvalue() or out.getvalue():
        print("self-test FAIL: incomplete report did not fail cleanly", file=sys.stderr)
        return 1

    # Integration of the actual preflight: prove `main()` itself returns 1
    # with a named cause when Valgrind is absent, without invoking a compiler
    # or printing an unearned PASS.  The mocked compiler resolver is confined
    # to this self-test; the real gate always verifies exact GCC/Clang/ICX
    # versions through `sv03_toolchain_identity`.
    import unittest.mock
    fake_rows = {name: {"path": "/bin/true"} for name in ("gcc16", "clang23")}
    with tempfile.TemporaryDirectory(prefix="lccc-best-tools-selftest-") as td:
        evidence = Path(td) / "evidence.json"
        out, err = io.StringIO(), io.StringIO()
        with unittest.mock.patch.object(
            identity, "require_compiler_versions", return_value=fake_rows
        ), unittest.mock.patch.object(
            sys.modules[__name__], "discover_tool",
            side_effect=lambda name: None if name == "valgrind" else "/bin/true",
        ), contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            preflight_rc = main(["--json", str(evidence)])
        if preflight_rc != 1 or out.getvalue() or "valgrind" not in err.getvalue():
            print("self-test FAIL: main() preflight did not fail cleanly", file=sys.stderr)
            return 1
        recorded = json.loads(evidence.read_text())
        if recorded.get("verdict") != "FAIL" or "valgrind" not in recorded.get("error", ""):
            print("self-test FAIL: preflight evidence was not recorded", file=sys.stderr)
            return 1
    print(f"PASS best-tools self-test ({len(mutations)} verdict mutations "
          f"rejected, fail-fast report paths proven, toolchain identity pinned)")
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
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()

    compilers = {"lccc": args.lccc, "gcc16": args.gcc, "clang23": args.clang}
    if args.icx:
        compilers["icx2026"] = args.icx
    elif args.require_icx:
        print("FAIL: --require-icx but no ICX path", file=sys.stderr)
        return 1

    report: dict = {
        "compilers": compilers,
        "require_icx": args.require_icx,
        "tools": {},
    }
    try:
        toolchain = identity.require_compiler_versions(
            compilers, search_paths=(CACHE_BIN,)
        )
        report["toolchain"] = toolchain
        pinned = {name: row["path"] for name, row in toolchain.items()}
        pinned["lccc"] = args.lccc
        for name in ("gdb", "objdump", "readelf", "valgrind", "objtool", "pahole", "perf"):
            report["tools"][name] = discover_tool(name)
        # The lane-plan audit reads the `command -v valgrind` / `gdb` idioms to
        # require provisioning.  Fail with a named cause instead of a bare
        # assert: an empty exception string and a later KeyError once hid the
        # real missing-tool failure behind an unearned success banner.
        for required in ("gdb", "objdump", "readelf", "valgrind"):
            if report["tools"].get(required) is None:
                raise RuntimeError(
                    f"required tool {required!r} is unavailable; provision it "
                    "before running the best-tool gate"
                )

        with tempfile.TemporaryDirectory(prefix="lccc-best-tools-") as td:
            work = Path(td)
            narrow = work / "narrow.c"
            narrow.write_text(NARROW)
            states = {
                "lccc_on": (pinned["lccc"], {}),
                "lccc_off": (pinned["lccc"], {"CCC_NO_COND_LOAD_SINK": "1"}),
                "gcc16": (pinned["gcc16"], {}),
                "clang23": (pinned["clang23"], {}),
            }
            if args.icx:
                states["icx2026"] = (pinned["icx2026"], {})
            static = {}
            asm_insns = {}
            for label, (cc, extra) in states.items():
                obj = work / f"{label}.o"
                compile_one(cc, narrow, obj, extra_env=extra)
                for fn in ("pick8", "pick16", "pick32", "pickf"):
                    n, insns = objdump_instructions(obj, fn)
                    static[f"{label}:{fn}"] = n
                    if label == "lccc_on" and fn == "pick8":
                        asm_insns[fn] = insns
            report["objdump"] = static
            report["gdb_asm_pick8"] = asm_insns.get("pick8")

            report["gdb"] = gdb_prot_none(args.lccc, work)

            fold = work / "fold.c"
            fold.write_text(BENCH_FOLD)
            report["ir_join_census"] = ir_join_census(args.lccc, fold, work)

            volatile = work / "volatile.c"
            volatile.write_text(BENCH_VOLATILE)
            refs = {}
            checksums = {}
            # Six Callgrind runs: fold ON/OFF (CCC_SIMPLIFY_SKIP), volatile
            # ON/OFF, plus GCC and Clang references on the foldable shape.
            jobs = [
                ("fold_on", args.lccc, fold, {}),
                ("fold_off", args.lccc, fold, {"CCC_SIMPLIFY_SKIP": "cast_sign_range"}),
                ("volatile_on", args.lccc, volatile, {}),
                ("volatile_off", args.lccc, volatile, {"CCC_SIMPLIFY_SKIP": "cast_sign_range"}),
                ("gcc16_fold", pinned["gcc16"], fold, {}),
                ("clang23_fold", pinned["clang23"], fold, {}),
            ]
            if args.icx:
                jobs.append(("icx2026_fold", pinned["icx2026"], fold, {}))
            for label, cc, src, extra in jobs:
                env = os.environ.copy()
                env.update(extra)
                exe = work / f"bench_{label}"
                cp = run([cc, "-O2", "-Wall", "-Wextra", "-Werror",
                          str(src), "-o", str(exe)], env=env)
                if cp.returncode != 0:
                    raise RuntimeError(f"{label} compile: {cp.stderr[-600:]}")
                direct = run([str(exe)])
                if direct.returncode != 0:
                    raise RuntimeError(f"{label} run: {direct.stderr}")
                checksums[label] = direct.stdout.strip()
                refs[label] = callgrind_refs(exe, work, label)
            report["callgrind"] = {
                "iterations": 100000,
                "refs": refs,
                "checksums": checksums,
            }
            report["structural"] = structural_readelf(work / "bench_fold_on")
            report["verdict"] = "PASS"
    except (RuntimeError, subprocess.TimeoutExpired, OSError, AssertionError) as exc:
        report["error"] = str(exc)
        report["verdict"] = "FAIL"

    report["problems"] = evaluate(report)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report, indent=2, sort_keys=True, default=str) + "\n")
    return report_evidence(report)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
