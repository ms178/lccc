#!/usr/bin/env python3
"""Generate and verify the AArch64 system-register table.

`src/backend/arm/assembler/encoder/sysreg_table.rs` holds the one table the
encoder resolves `mrs <Xt>,<name>` and `msr <name>,<Xt>` through.  Its names
and encodings come from binutils' own `opcodes/aarch64-sys-regs.def` -- the
exact table GNU as assembles these names with -- so the compiler and the
assembler cannot disagree about what a register is called or which system
register it names.  The table used to be two hand-written `match` blocks in
`system.rs` (115 of the 1619 names binutils knows, and the two blocks had
already drifted apart); a register the compiler does not know is a hard error
in `mrs`/`msr`, which is precisely the code that kernel-style C reaches for.

Three verdicts are held together, and `--check` fails if any pair disagrees:

  * the .def file (names + CPENC(op0,op1,CRn,CRm,op2) as binutils defines it),
  * the pinned GNU as (what the name actually assembles to, in both
    directions, under the same `.arch` prologue the rest of the AArch64
    gates use), and
  * the Rust table text, plus -- when a compiler binary is given -- the words
    the compiler itself emits for the whole list.

The whole list is assembled in ONE GNU as invocation per direction: as reports
every bad line and keeps going, so the rejects come from the error lines and
the accepted encodings from a second pass of just those names (object bytes in
name order).  1619 names therefore cost two assembler runs, not 3238.

Usage:
  aarch64_sysreg_table.py --check [--as AS] [--objcopy OBJCOPY]
                                [--lccc BINARY] [--def FILE] [--tarball FILE]
  aarch64_sysreg_table.py --regenerate [--def FILE] [--tarball FILE] [--as AS]
                                       [--objcopy OBJCOPY] [--out FILE]

Exit codes: 0 = consistent, 1 = inconsistent, 2 = tooling missing (a caller can
then distinguish "wrong" from "cannot tell", the same contract the operand
legality matrix uses).
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_OUT = REPO / "src/backend/arm/assembler/encoder/sysreg_table.rs"
DEF_MEMBER = "binutils-2.47/opcodes/aarch64-sys-regs.def"
BINUTILS_VERSION = "2.47"

# The `.arch` directive the AArch64 oracle gates assemble under; the table is
# only meaningful relative to the same feature set GNU as is asked about.
PROLOGUE = ".arch armv9.4-a+sme\n.text\n"

# PSTATE fields are NOT in aarch64-sys-regs.def: the ones that can be read or
# written with a register operand (`mrs x0,pan`) live in binutils'
# `aarch64_pstatefields` table.  daifset/daifclr are listed here so the check
# proves they have no register form at all rather than quietly omitting them;
# svcrsm/svcrza/svcrsmza are immediate-only (SME) for the same reason.
PSTATE_CANDIDATES = (
    "spsel", "daifset", "daifclr", "pan", "uao", "allint",
    "ssbs", "dit", "tco", "nzcv", "daif", "currentel",
    "svcrsm", "svcrza", "svcrsmza", "dze", "pm",
)

DEF_ROW = re.compile(
    r'SYSREG\s*\(\s*"([a-z0-9_]+)"\s*,\s*CPENC\s*\((\d+)\s*,\s*(\d+)\s*,'
    r'\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*\)'
)
RUST_ROW = re.compile(r'^\s*\("([a-z0-9_]+)",\s*(0x[0-9a-f]{4})\),\s*$', re.MULTILINE)


def die(msg: str, code: int = 1) -> "NoReturn":  # noqa: F821
    print(f"aarch64_sysreg_table: {msg}", file=sys.stderr)
    raise SystemExit(code)


def sysreg_encoding(op0: int, op1: int, crn: int, crm: int, op2: int) -> int:
    return ((op0 & 3) << 14) | ((op1 & 7) << 11) | ((crn & 0xF) << 7) \
        | ((crm & 0xF) << 3) | (op2 & 7)


def find_def(explicit_def: str | None, tarball: str | None) -> tuple[list[str], dict[str, int]]:
    """(names, name -> encoding) from binutils' own system-register table."""
    text = None
    if explicit_def:
        text = Path(explicit_def).read_text()
    else:
        candidates = []
        if tarball:
            candidates.append(Path(tarball))
        dl = Path(os.environ.get("GAS_DL_DIR", str(Path.home() / "dl")))
        candidates.append(dl / f"binutils-{BINUTILS_VERSION}.tar.xz")
        candidates.append(Path(f"/var/tmp/{BINUTILS_VERSION}.tar.xz"))
        for cand in candidates:
            if not cand.is_file():
                continue
            with tarfile.open(cand, "r:xz") as tf:
                member = tf.extractfile(DEF_MEMBER)
                if member is None:
                    die(f"{cand} has no {DEF_MEMBER}", 2)
                text = member.read().decode()
            break
    if text is None:
        die("no binutils sys-regs.def: pass --def, or keep the pinned "
            f"binutils-{BINUTILS_VERSION}.tar.xz in GAS_DL_DIR", 2)

    names: list[str] = []
    encodings: dict[str, int] = {}
    for m in DEF_ROW.finditer(text):
        name, fields = m.group(1), [int(x) for x in m.groups()[1:]]
        if name in encodings:
            die(f"{name} appears twice in the .def table")
        names.append(name)
        encodings[name] = sysreg_encoding(*fields)
    if len(names) < 1000:
        die(f"parsed only {len(names)} rows out of the .def table; the file "
            "must have moved, refusing to emit a smaller table", 2)
    return names, encodings


def batch_assemble(names: list[str], template: str, as_bin: str, objcopy: str,
                   tmp: Path) -> tuple[list[str], dict[str, int]]:
    """Assemble one instruction per name, in one run, and read the verdicts.

    GNU as reports every failing line and continues, and `objcopy` then yields
    the encodings of the surviving lines in order -- so one assembler run gives
    both the reject set (from the message lines) and the encodings.
    """
    src = tmp / "sweep.s"
    obj = tmp / "sweep.o"
    binf = tmp / "sweep.bin"
    src.write_text(PROLOGUE + "".join(template.format(n) + "\n" for n in names))
    for f in (obj, binf):
        if f.exists():
            f.unlink()
    r = subprocess.run([as_bin, "-o", str(obj), str(src)],
                       capture_output=True, text=True)
    rejected = {int(m.group(1)) - 3 for m in
                re.finditer(r"sweep\.s:(\d+): Error", r.stderr)}
    # The attribution is only sound when the exit status and the diagnostics
    # agree.  Both mismatches matter: `as` can report a failing line and still
    # exit 0 (the previous single-condition test silently accepted that,
    # because it only rejected the *opposite* pairing), and it can exit
    # non-zero with no attributable line at all -- in which case every name
    # would look accepted to the line-number arithmetic below.
    if not r.returncode and rejected:
        die("GNU as exited 0 but reported errors on lines "
            + ", ".join(str(i) for i in sorted(rejected)[:8])
            + "; the verdict attribution cannot be trusted")
    if r.returncode and not rejected:
        die("GNU as failed without reporting a line; cannot attribute verdicts")
    accepted = [n for i, n in enumerate(names) if i not in rejected]
    if not accepted:
        return [], {}
    if rejected:
        # GNU as wrote no object for a file it diagnosed, so the encodings come
        # from a second run of just the surviving names (same order).
        src.write_text(PROLOGUE + "".join(template.format(n) + "\n" for n in accepted))
        subprocess.run([as_bin, "-o", str(obj), str(src)], check=True,
                       capture_output=True)
    subprocess.run([objcopy, "-O", "binary", "--only-section=.text", str(obj),
                    str(binf)], check=True, capture_output=True)
    data = binf.read_bytes()
    if len(data) != 4 * len(accepted):
        die(f"assembled {len(accepted)} instructions but got {len(data)} bytes")
    return accepted, {n: int.from_bytes(data[4 * i:4 * i + 4], "little")
                      for i, n in enumerate(accepted)}


def probe_both_directions(names: list[str], as_bin: str, objcopy: str,
                          tmp: Path) -> dict[str, int]:
    """name -> encoding, for names GNU as accepts in BOTH directions."""
    read_ok, read_w = batch_assemble(names, "mrs x0,{}", as_bin, objcopy, tmp)
    write_ok, write_w = batch_assemble(names, "msr {},x0", as_bin, objcopy, tmp)
    out: dict[str, int] = {}
    for name in read_ok:
        if name not in write_ok:
            continue
        enc = (read_w[name] >> 5) & 0xFFFF
        if enc != ((write_w[name] >> 5) & 0xFFFF):
            die(f"GNU as reads {name} as {enc:#06x} but writes it as "
                f"{(write_w[name] >> 5) & 0xFFFF:#06x}")
        out[name] = enc
    return out


def parse_rust_table(path: Path) -> dict[str, int]:
    if not path.is_file():
        die(f"{path} does not exist", 2)
    rows = dict(RUST_ROW.findall(path.read_text()))
    return {k: int(v, 16) for k, v in rows.items()}


def render_table(encodings: dict[str, int]) -> str:
    body = "".join(f'    ("{n}", 0x{encodings[n]:04x}),\n' for n in sorted(encodings))
    return f'''//! System registers, as GNU as knows them.
//!
//! GENERATED by `scripts/aarch64_sysreg_table.py --regenerate` from the pinned
//! binutils {BINUTILS_VERSION} `opcodes/aarch64-sys-regs.def` -- the table GNU as
//! itself assembles these names with -- and verified against GNU as and this
//! crate's own encoder by `--check`.  Do not hand-edit: a new binutils brings a
//! new table, and the script is the way to re-derive it.
//!
//! The table is direction-agnostic on purpose (one row serves `mrs` and `msr`)
//! and sorted by name, so `SYSREGS.binary_search_by` finds a row in log2(n)
//! comparisons without a map.  Only names GNU as accepts in BOTH directions are
//! emitted, so `--check` has one verdict per name to hold the table against and
//! the operand-legality matrix's `sweep system sysreg` group -- which
//! enumerates this list -- cannot contain a row the assembler disagrees with.
//!
//! What this replaces: a hand-written pair of `match` arms in `system.rs`
//! covering 115 of the {len(encodings)} names, which had drifted apart, and
//! which rejected the rest with "unsupported system register" -- including
//! `mrs x0,cntv_cval_el0`'s sibling encodings that were simply wrong.

/// Every system register GNU as accepts under `.arch armv9.4-a+sme`, with the
/// 16-bit `op0:op1:CRn:CRm:op2` encoding its name stands for.
pub(crate) const SYSREGS: &[(&str, u32)] = &[
{body}];
'''


def resolve_lccc(explicit: str | None) -> str | None:
    """The compiler to cross-check, or None when none was asked for.

    An *explicit* path that does not exist is a hard error, not a skip: the
    caller asked for the encoder to be verified, and silently verifying only
    GNU as against itself is the failure mode this whole tool exists to
    prevent (the ci_local gate passes `--lccc target/fastbuild/lccc`, so a
    mis-built tree would otherwise report a clean bill of health).
    """
    if explicit:
        if not Path(explicit).is_file():
            die(f"--lccc {explicit} does not exist; refusing to check the "
                "table against GNU as alone when the encoder was requested", 2)
        return explicit
    for cand in ("target/fastbuild/lccc", "target/fastbuild/lccc-arm"):
        p = REPO / cand
        if p.is_file():
            return str(p)
    return None


def _lccc_sweep(insns: list[str], lccc: str, objcopy: str,
                tmp: Path) -> tuple[bool, str, bytes]:
    """Assemble one instruction per line with this crate; return (ok, err, bytes).

    The whole sweep goes through one compiler run -- 3,238 spawns per direction
    would dominate the gate -- and `objcopy` (the *caller's*, so a pinned
    toolchain stays pinned) reads the words back out.
    """
    link = tmp / "aarch64-linux-gnu-ccc"
    if not link.exists():
        link.symlink_to(Path(lccc).resolve())
    src = tmp / "lccc.s"
    obj = tmp / "lccc.o"
    binf = tmp / "lccc.bin"
    src.write_text(PROLOGUE + "".join(i + "\n" for i in insns))
    for f in (obj, binf):
        if f.exists():
            f.unlink()
    r = subprocess.run([str(link), "-c", str(src), "-o", str(obj)],
                       capture_output=True, text=True)
    if r.returncode:
        return False, r.stderr.strip(), b""
    subprocess.run([objcopy, "-O", "binary", "--only-section=.text",
                    str(obj), str(binf)], check=True, capture_output=True)
    data = binf.read_bytes()
    if len(data) != 4 * len(insns):
        return False, (f"assembled {len(insns)} instructions but emitted "
                       f"{len(data)} bytes"), b""
    return True, "", data


def _bisect_failing_line(insns: list[str], lccc: str, objcopy: str,
                         tmp: Path) -> int | None:
    """Index of the first line this crate refuses, by binary search.

    A file-level failure carries no line number in this compiler's
    diagnostics, so the failing prefix is found by bisection (log2(3238) ~= 12
    runs) instead of a per-line loop.  None when the failure is not
    attributable to a single line.
    """
    lo, hi = 0, len(insns)  # insns[:hi] fails, insns[:lo] assembles
    ok, _, _ = _lccc_sweep(insns[:1], lccc, objcopy, tmp)
    if ok:
        return None
    while hi - lo > 1:
        mid = (lo + hi) // 2
        ok, _, _ = _lccc_sweep(insns[:mid], lccc, objcopy, tmp)
        if ok:
            lo = mid
        else:
            hi = mid
    return lo


def run_lccc(words_path: dict[str, int], lccc: str, objcopy: str,
             names: list[str], tmp: Path) -> list[str]:
    """Verify this crate's encoder against the table, in BOTH directions.

    `mrs` and `msr` reach the same resolver through the same table but were
    historically separate hand-written maps that drifted apart (`mrs x0,pan`
    was rejected while `msr pan,#1` was accepted, and 28 registers were
    readable but not writable), so a checker that only reads is blind to half
    the bug class.  Each direction is swept as one file; when the compiler
    refuses a line the whole run stops, so the failing name is isolated by
    bisection and the sweep continues after it, which yields a per-register
    verdict instead of one opaque "the sweep file was refused".
    """
    problems: list[str] = []
    for template, label in (("mrs x0,{}", "mrs"), ("msr {},x0", "msr")):
        todo = list(range(len(names)))
        rounds = 0
        while todo:
            rounds += 1
            if rounds > 64:
                problems.append(
                    f"{label}: more than 64 failing lines; stopping the sweep "
                    f"({len(todo)} names unchecked)")
                break
            sel = [names[i] for i in todo]
            ok, err, data = _lccc_sweep([template.format(n) for n in sel],
                                        lccc, objcopy, tmp)
            if ok:
                for k, i in enumerate(todo):
                    word = int.from_bytes(data[4 * k:4 * k + 4], "little")
                    want = words_path.get(names[i], -1)
                    got = (word >> 5) & 0xFFFF
                    if got != want:
                        problems.append(
                            f"{label} {names[i]}: lccc {got:#06x} != table "
                            f"{want:#06x}")
                break
            bad = _bisect_failing_line([template.format(n) for n in sel],
                                       lccc, objcopy, tmp)
            if bad is None:
                first = err.splitlines()[0] if err else "no diagnostics"
                problems.append(f"{label}: lccc refused the sweep ({first})")
                break
            problems.append(f"{label} {sel[bad]}: lccc rejects it")
            todo = todo[bad + 1:]
    return problems


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--regenerate", action="store_true")
    ap.add_argument("--as", dest="as_bin", default=None,
                    help="GNU as 2.47 (default: aarch64-linux-gnu-as)")
    ap.add_argument("--objcopy", default=None)
    ap.add_argument("--lccc", default=None,
                    help="compiler binary to verify too (--check only)")
    ap.add_argument("--def", dest="def_file", default=None,
                    help="aarch64-sys-regs.def")
    ap.add_argument("--tarball", default=None, help="pinned binutils tarball")
    ap.add_argument("--out", default=str(DEFAULT_OUT))
    args = ap.parse_args()

    as_bin = args.as_bin or shutil.which("aarch64-linux-gnu-as")
    objcopy = args.objcopy or shutil.which("aarch64-linux-gnu-objcopy")
    names, def_enc = find_def(args.def_file, args.tarball)
    candidates = names + [n for n in PSTATE_CANDIDATES if n not in def_enc]

    if args.regenerate:
        if not as_bin or not objcopy:
            die("--regenerate needs GNU as and objcopy", 2)
        with tempfile.TemporaryDirectory() as td:
            got = probe_both_directions(candidates, as_bin, objcopy, Path(td))
        for name, enc in got.items():
            if name in def_enc and def_enc[name] != enc:
                die(f"{name}: .def says {def_enc[name]:#06x}, GNU as emits {enc:#06x}")
        missing = [n for n in candidates if n not in got]
        Path(args.out).write_text(render_table(got))
        print(f"wrote {args.out}: {len(got)} registers "
              f"({len(names)} from the .def table, "
              f"{len(got) - len([n for n in got if n in def_enc])} PSTATE fields); "
              f"rejected in at least one direction: {len(missing)}")
        if missing:
            print("  not emitted: " + ", ".join(missing[:20])
                  + (" ..." if len(missing) > 20 else ""))
        return 0

    # --check
    table = parse_rust_table(Path(args.out))
    if not as_bin or not objcopy:
        die("--check needs GNU as and objcopy", 2)
    with tempfile.TemporaryDirectory() as td:
        got = probe_both_directions(candidates, as_bin, objcopy, Path(td))
    problems: list[str] = []
    for name, enc in sorted(got.items()):
        if name not in table:
            problems.append(f"{name}: GNU as accepts it, the table is missing it")
        elif table[name] != enc:
            problems.append(f"{name}: table {table[name]:#06x}, GNU as {enc:#06x}")
    for name in sorted(table):
        if name not in got:
            problems.append(f"{name}: in the table, GNU as rejects it")
    for name in names:
        if name in def_enc and def_enc[name] != got.get(name, def_enc[name]):
            problems.append(f"{name}: .def and GNU as disagree")
    lccc = resolve_lccc(args.lccc)
    if lccc and not problems:
        with tempfile.TemporaryDirectory() as td:
            problems += run_lccc(table, lccc, objcopy, sorted(table), Path(td))
    if problems:
        print(f"aarch64_sysreg_table: {len(problems)} problem(s)", file=sys.stderr)
        for p in problems[:40]:
            print("  " + p, file=sys.stderr)
        return 1
    print(f"aarch64_sysreg_table: {len(table)} registers agree with GNU as"
          + (f" and {lccc}" if lccc else ""))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
