#!/usr/bin/env python3
"""D6 inline-instance gate: abstract origins, concrete ranges, nesting, GDB.

The writer under test records one inline instance per cloned callee body and
pairs zero-width code markers into address ranges.  This gate compiles source
it wrote itself, decodes the resulting DWARF with the shared quality reader,
and asks a real debugger to resolve the inline frames.  It checks both delivery
paths (direct object emission and `-S -g` assembly) and every DWARF version the
compiler claims, on every target binary supplied.

Negative controls are part of the contract, not decoration: `-g0` states no
DWARF, `-O0` does not inline the ordinary probe, a `noinline` callee keeps a
concrete subprogram and no inline DIE, and a fully optimized-away body states no
zero-width inline range, and `CCC_VERIFY_IR=abort` proves the optimizing
passes keep instruction/span/inline-id arrays parallel (and that no `-g` means
no inline ids or DWARF).  `--self-test` runs those controls against the host
compiler and exercises the range-list decoder mutations in the shared reader.

Usage:
    python3 tests/regression/check_dwarf_inlining.py --lccc target/fastbuild/lccc
    python3 tests/regression/check_dwarf_inlining.py --lccc ... --gdb gdb --json out.json
    python3 tests/regression/check_dwarf_inlining.py --self-test --lccc ...
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_dwarf_quality as q  # noqa: E402

DW_AT_INLINE = 0x20
DW_AT_CALL_FILE = 0x58
DW_AT_CALL_LINE = 0x59
DW_AT_CALL_COLUMN = 0x57
DW_AT_PROTOTYPED = 0x27
DW_INL_INLINED = 1
DW_INL_DECLARED_NOT_INLINED = 2
DW_INL_DECLARED_INLINED = 3

NESTED = """\
extern int seed(void);
static inline int leaf(int x) { int y = x + 1; return y * 2; }
static int middle(int x) { int a = leaf(x); int b = leaf(a); return a + b; }
int main(void) {
    int x = seed();
    int a = middle(x);
    int b = middle(x + 3);
    return a + b == 12 * x + 34 ? 0 : 1;
}
"""
SEED = "int seed(void) { return 7; }\n"
NOINLINE = """\
extern int seed(void);
__attribute__((noinline)) static int kept(int x) { return x + 1; }
int main(void) { return kept(seed()) == 8 ? 0 : 1; }
"""
DEAD = """\
extern int seed(void);
static inline int gone(int x) { return x + 1; }
int main(void) { int x = seed(); (void)gone(x); return x == 7 ? 0 : 1; }
"""
VOID_KR = """\
extern int seed(void);
static inline void touch(int x) { if (x) seed(); }
static int old_style(x) int x; { return x + 1; }
int main(void) { int x = seed(); touch(x); return old_style(x) == 8 ? 0 : 1; }
"""


class Failure(Exception):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise Failure(message)


def run(cmd: list[str], cwd: Path, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, cwd=cwd, text=True, capture_output=True, timeout=120, env=env)


def compile_source(lccc: str, src: Path, out: Path, cwd: Path, *flags: str) -> None:
    proc = run([lccc, "-std=c23", *flags, str(src), "-o", str(out)], cwd)
    require(proc.returncode == 0, f"compile failed: {' '.join(proc.args)}\n{proc.stderr}")


def die_ranges(elf: q.Elf, die: q.Die, lists: list[q.RangeList]) -> list[tuple[int, int]]:
    offset = die.attrs.get(q.DW_AT_RANGES)
    if isinstance(offset, int):
        return q.ranges_for(elf, lists, offset)
    low = die.attrs.get(q.DW_AT_LOW_PC)
    high = die.attrs.get(q.DW_AT_HIGH_PC)
    require(isinstance(low, int) and isinstance(high, int), "inline DIE states no range")
    if die.forms.get(q.DW_AT_HIGH_PC, q.DW_FORM_ADDR) != q.DW_FORM_ADDR:
        high = low + high
    return [(low, high)]


def parent_of(unit: q.Unit, die: q.Die) -> q.Die | None:
    parent = None
    for candidate in unit.dies:
        if candidate.offset >= die.offset:
            break
        if candidate.depth == die.depth - 1:
            parent = candidate
    return parent


def check_object(path: Path, version: int, expect: set[str], forbid: set[str] = frozenset()) -> dict:
    elf = q.read_elf(path)
    units = q.units(elf)
    require(units, f"{path}: no DWARF units")
    lists = q.range_lists(elf, version)
    found: set[str] = set()
    abstracts: dict[str, q.Die] = {}
    inline_count = 0
    nested_count = 0
    for unit in units:
        for die in unit.by_tag(q.DW_TAG_SUBPROGRAM):
            if q.DW_AT_LOW_PC not in die.attrs and DW_AT_INLINE in die.attrs:
                name = q.die_name(die, unit)
                value = die.attrs[DW_AT_INLINE]
                require(value in (DW_INL_INLINED, DW_INL_DECLARED_INLINED),
                        f"{path}: abstract {name} has DW_AT_inline {value}, not an inlined encoding")
                abstracts[name] = die
        concrete = [(low, high) for _, low, high, _ in q.subprogram_ranges(elf, unit, version)]
        for die in unit.by_tag(q.DW_TAG_INLINED_SUBROUTINE):
            inline_count += 1
            name = q.die_name(die, unit)
            found.add(name)
            origin = q.reference_target(die, unit, q.DW_AT_ABSTRACT_ORIGIN)
            require(origin is not None and origin.tag == q.DW_TAG_SUBPROGRAM,
                    f"{path}: inline {name} has no subprogram abstract origin")
            require(DW_AT_INLINE in origin.attrs, f"{path}: abstract {name} lacks DW_AT_inline")
            ranges = die_ranges(elf, die, lists)
            require(ranges, f"{path}: inline {name} has an empty range list")
            for start, end in ranges:
                require(start < end, f"{path}: inline {name} states zero-width [{start:#x},{end:#x})")
                require(any(low <= start and end <= high for low, high in concrete),
                        f"{path}: inline {name} [{start:#x},{end:#x}) escapes every concrete function")
            parent = parent_of(unit, die)
            if parent is not None and parent.tag == q.DW_TAG_INLINED_SUBROUTINE:
                nested_count += 1
                outer = die_ranges(elf, parent, lists)
                for start, end in ranges:
                    require(any(a <= start and end <= b for a, b in outer),
                            f"{path}: inline {name} escapes its inline parent")
            if DW_AT_CALL_FILE in die.attrs:
                require(isinstance(die.attrs.get(DW_AT_CALL_LINE), int) and die.attrs[DW_AT_CALL_LINE] > 0,
                        f"{path}: inline {name} states a call file without a call line")
    require(expect <= found, f"{path}: missing inline instances {sorted(expect - found)}; found {sorted(found)}")
    require(not (forbid & found), f"{path}: forbidden inline instances {sorted(forbid & found)}")
    require(expect <= set(abstracts), f"{path}: missing abstract origins {sorted(expect - set(abstracts))}")
    return {"inline_dies": inline_count, "nested_dies": nested_count, "names": sorted(found)}


def check_no_inline(path: Path, name: str) -> None:
    elf = q.read_elf(path)
    for unit in q.units(elf):
        for die in unit.by_tag(q.DW_TAG_INLINED_SUBROUTINE):
            require(q.die_name(die, unit) != name, f"{path}: {name} was inlined despite noinline")
        require(any(die.tag == q.DW_TAG_SUBPROGRAM and q.die_name(die, unit) == name
                    and q.DW_AT_LOW_PC in die.attrs for die in unit.dies),
                f"{path}: noinline {name} lost its concrete subprogram")


def gdb_inline_frames(gdb: str, exe: Path, cwd: Path) -> dict:
    script = [
        "set pagination off", "set confirm off", "break leaf", "run", "bt",
        "info frame", "next", "frame", "disable 1", "break middle", "continue", "bt",
    ]
    cmd = [gdb, "-q", "-batch"]
    for item in script:
        cmd += ["-ex", item]
    cmd.append(str(exe))
    proc = run(cmd, cwd)
    text = proc.stdout + proc.stderr
    require(proc.returncode == 0, f"gdb failed:\n{text}")
    require("Breakpoint" in text and "leaf" in text, f"gdb did not resolve leaf:\n{text}")
    for frame in ("leaf", "middle", "main"):
        require(frame in text, f"gdb backtrace lacks {frame}:\n{text}")
    require("inlined into frame" in text, f"gdb did not report an inline frame:\n{text}")
    require("nested.c:2" in text, f"gdb did not stop at the leaf source line:\n{text}")
    return {"gdb": "inline frames, breakpoint and step verified"}


def structural_matrix(args: argparse.Namespace, work: Path) -> list[dict]:
    rows = []
    targets = [("x86-64", args.lccc)]
    if args.lccc_i686:
        targets.append(("i686", args.lccc_i686))
    if args.lccc_arm:
        targets.append(("aarch64", args.lccc_arm))
    if args.lccc_riscv:
        targets.append(("riscv", args.lccc_riscv))
    nested = work / "nested.c"
    nested.write_text(NESTED)
    for target, lccc in targets:
        for version in (2, 3, 4, 5):
            obj = work / f"nested-{target}-{version}.o"
            compile_source(lccc, nested, obj, work, "-O2", "-g", f"-gdwarf-{version}", "-c")
            measured = check_object(obj, version, {"leaf", "middle"})
            rows.append({"target": target, "version": version, "kind": "nested", **measured})
        obj = work / f"nested-{target}-o0.o"
        compile_source(lccc, nested, obj, work, "-O0", "-g", "-c")
        check_object(obj, q.units(q.read_elf(obj))[0].version, set(), forbid={"leaf", "middle"})
        rows.append({"target": target, "version": "default", "kind": "O0-negative", "inline_dies": 0})
    return rows


def host_controls(args: argparse.Namespace, work: Path) -> list[dict]:
    rows = []
    lccc = args.lccc
    seed = work / "seed.c"
    seed.write_text(SEED)
    nested = work / "nested.c"
    nested.write_text(NESTED)
    noinline = work / "noinline.c"
    noinline.write_text(NOINLINE)
    dead = work / "dead.c"
    dead.write_text(DEAD)
    void_kr = work / "void_kr.c"
    void_kr.write_text(VOID_KR)

    obj = work / "noinline.o"
    compile_source(lccc, noinline, obj, work, "-O2", "-g", "-c")
    check_no_inline(obj, "kept")
    rows.append({"kind": "noinline-negative", "result": "concrete subprogram, no inline DIE"})

    obj = work / "dead.o"
    compile_source(lccc, dead, obj, work, "-O2", "-g", "-c")
    check_object(obj, q.units(q.read_elf(obj))[0].version, set(), forbid={"gone"})
    rows.append({"kind": "optimized-away-negative", "result": "no zero-width inline range"})

    obj = work / "void_kr.o"
    compile_source(lccc, void_kr, obj, work, "-O2", "-g", "-c")
    elf = q.read_elf(obj)
    for unit in q.units(elf):
        for die in unit.by_tag(q.DW_TAG_SUBPROGRAM):
            name = q.die_name(die, unit)
            if name == "touch" and DW_AT_INLINE in die.attrs:
                require(q.DW_AT_TYPE not in die.attrs, "void inline abstract origin invented a return type")
            if name == "old_style" and DW_AT_INLINE in die.attrs:
                require(DW_AT_PROTOTYPED not in die.attrs, "K&R inline abstract origin claimed a prototype")
    rows.append({"kind": "void-kr-metadata", "result": "no invented void type or K&R prototype"})

    g0 = work / "nested-g0.o"
    compile_source(lccc, nested, g0, work, "-O2", "-g0", "-c")
    require(".debug_info" not in q.read_elf(g0).sections, "-g0 emitted .debug_info")
    rows.append({"kind": "g0-negative", "result": "no DWARF"})

    # The IR verifier is the tripwire for provenance drift: every optimizing
    # pass must keep instructions/spans/inline ids parallel, and without -g the
    # inliner must not allocate ids at all.  Abort mode turns a drifted array
    # into a compile failure instead of a silent debug-info degradation.
    verify_env = dict(os.environ, CCC_VERIFY_IR="abort")
    targets = [("x86-64", args.lccc)]
    if args.lccc_i686:
        targets.append(("i686", args.lccc_i686))
    if args.lccc_arm:
        targets.append(("aarch64", args.lccc_arm))
    if args.lccc_riscv:
        targets.append(("riscv64", args.lccc_riscv))
    for name, compiler in targets:
        for opt in ("-O1", "-O2"):
            for debug in (False, True):
                flags = [opt, "-c"] + (["-g"] if debug else [])
                out = work / f"verify-{name}-{opt}-{'g' if debug else 'nog'}.o"
                proc = run([compiler, "-std=c23", *flags, str(nested), "-o", str(out)], work, verify_env)
                require(proc.returncode == 0,
                        f"IR verifier aborted for {name} {opt} {'-g' if debug else 'no -g'}:\n{proc.stderr}")
                if not debug:
                    require(".debug_info" not in q.read_elf(out).sections,
                            f"{name} {opt} without -g emitted .debug_info")
        rows.append({"kind": "provenance-verifier", "target": name,
                     "result": "O1/O2 with and without -g keep IR provenance parallel"})

    if args.gdb:
        exe = work / "nested.elf"
        compile_source(lccc, nested, exe, work, "-O2", "-g", "-fno-omit-frame-pointer", str(seed))
        proc = run([str(exe)], work)
        require(proc.returncode == 0, f"nested executable returned {proc.returncode}")
        rows.append(gdb_inline_frames(args.gdb, exe, work))
        for version in (4, 5):
            asm = work / f"nested-{version}.s"
            proc = run([lccc, "-std=c23", "-O2", "-g", f"-gdwarf-{version}", "-S", str(nested), "-o", str(asm)], work)
            require(proc.returncode == 0, f"-S delivery failed:\n{proc.stderr}")
            require(".Llccc_inline_" in asm.read_text(), "-S assembly lost inline markers")
            gas_obj = work / f"nested-{version}-gas.o"
            assembler = args.gcc or shutil.which("gcc")
            require(assembler is not None, "no assembler for -S delivery")
            proc = run([assembler, "-c", str(asm), "-o", str(gas_obj)], work)
            require(proc.returncode == 0, f"GAS rejected -S debug payload:\n{proc.stderr}")
            measured = check_object(gas_obj, version, {"leaf", "middle"})
            rows.append({"kind": f"gas-delivery-v{version}", **measured})
    return rows


def self_test(args: argparse.Namespace, work: Path) -> None:
    # Decoder mutation: the v5 start_end/base_address/offset_pair contract is
    # exercised by the shared reader's own self-test; run it here so this gate
    # cannot pass on a reader that misdecodes D6 range lists.
    proc = run([sys.executable, str(Path(__file__).with_name("check_dwarf_quality.py")), "--self-test"], work)
    require(proc.returncode == 0, f"shared decoder self-test failed:\n{proc.stdout}{proc.stderr}")
    rows = host_controls(args, work)
    require(any(row["kind"] == "noinline-negative" for row in rows), "self-test lost its negative control")
    print("self-test: decoder mutations and source-level negative controls agree")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", required=True)
    parser.add_argument("--lccc-i686")
    parser.add_argument("--lccc-arm")
    parser.add_argument("--lccc-riscv")
    parser.add_argument("--gdb")
    parser.add_argument("--gcc")
    parser.add_argument("--json")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    # Tool paths are relative to the caller's cwd (the repository), but every
    # compile runs inside a temporary directory; resolve them before chdir.
    for name in ("lccc", "lccc_i686", "lccc_arm", "lccc_riscv", "gdb", "gcc"):
        value = getattr(args, name)
        if value:
            if os.path.sep in value or Path(value).exists():
                setattr(args, name, str(Path(value).resolve()))
            else:
                resolved = shutil.which(value)
                require(resolved is not None, f"tool not found: {value}")
                setattr(args, name, resolved)
    with tempfile.TemporaryDirectory(prefix="dwarf-inline-") as tmp:
        work = Path(tmp)
        try:
            if args.self_test:
                self_test(args, work)
                return 0
            rows = structural_matrix(args, work) + host_controls(args, work)
        except Failure as error:
            print(f"FAIL: {error}", file=sys.stderr)
            return 1
    payload = {"gate": "dwarf_inlining", "rows": rows, "failures": 0}
    if args.json:
        Path(args.json).write_text(json.dumps(payload, indent=2) + "\n")
    print(f"dwarf inlining: {len(rows)} checks passed")
    for row in rows:
        print("  " + json.dumps(row, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
