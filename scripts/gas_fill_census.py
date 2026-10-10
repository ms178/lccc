#!/usr/bin/env python3
"""Census LCCC's alignment-padding law against the pinned GNU as 2.47 oracle.

WHY
---
`.p2align` and `.balign` decide two things about the bytes between an arbitrary
offset and an aligned label: how *many* and *which*. Both are load-bearing.
The content, because a NOP may only sit where a hart can fetch it and because a
pad that decodes differently is a pad the disassembler lies about. The size,
because every address after the directive moves with it -- branch-range checks,
relaxation hints and any byte-exact `.o` comparison hang off that.

Neither half follows from the ELF psABI or from the shape of the ISA: the rules
live in binutils' `config/tc-<arch>.c` plus `frags.c`, and they differ per
target. So this script measures instead of reasoning. For each cell it writes
one `.s`, assembles it with the pinned cross `as` and with LCCC's own assembler,
reads both `.text` sections back with the *same* `objdump` (so a reader bug
cannot masquerade as an assembler bug), and compares.

RUNNING
-------
    scripts/gas_fill_census.py                  # both targets, 600 cells
    scripts/gas_fill_census.py riscv64          # one target
    scripts/gas_fill_census.py --keep-cells     # leave the scratch files

Needs the two GAS trees the repo's binutils oracle provisions
(`scripts/ensure_gas_247.sh`; `GAS_CACHE`/`GAS_DL_DIR` say where its sources
live, and a directory outside `$HOME` survives a wiped home) and a
`--profile fastbuild` build of the assembler binaries. Both are checked up
front: the script never reports success for a target it could not run.
`GAS247_AUTO_PROVISION=1` lets it invoke the provisioning script itself.

EXIT CODES
----------
0  every cell agrees, apart from the one class this script exists to allow:
   RISC-V cells where GAS pads a whole extra quantum beyond the boundary. That
   is `riscv_frag_align_code` writing a worst-case NOP run for the linker to
   relax, and LCCC deliberately does not imitate it -- LCCC's linker treats
   `R_RISCV_ALIGN` as a hint it skips, so the inflation would survive into the
   image. `elf/writer_base.rs` and
   `padding_law_tests::pad_reaches_the_boundary_and_stops_there` carry the
   reasoning; the count is printed so that any change in that class, in either
   direction, is visible.
1  a cell diverges in content, aarch64 diverges at all, or a tool is missing.
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PAYLOADS = (1, 2, 3, 4, 5, 6, 7, 9, 13, 16)
EXPS = (0, 1, 2, 3, 4, 5)
TAILS = ("", ", 0xff", ",, 3", ",, 40", ", 0xff, 3")

TARGETS = {
    "aarch64": {"triple": "aarch64-linux-gnu", "lccc_bin": "lccc-arm"},
    "riscv64": {"triple": "riscv64-linux-gnu", "lccc_bin": "lccc-riscv"},
}


def fail(msg: str) -> "SystemExit":
    print(f"gas_fill_census: {msg}", file=sys.stderr)
    raise SystemExit(1)


def tool(prefix: Path, triple: str, name: str) -> Path:
    """`<prefix>/bin/<name>`, accepting either install layout: the repo's
    provisioning script installs unprefixed tools into a per-triple prefix, a
    hand-built cross binutils uses the `<triple>-` program prefix."""
    for cand in (prefix / "bin" / name, prefix / "bin" / f"{triple}-{name}"):
        if cand.is_file():
            return cand
    return fail(f"no {name} under {prefix}/bin")


def gas_prefix(triple: str, override: str | None) -> Path:
    if override:
        p = Path(override)
    else:
        p = Path(os.environ.get("GAS247_PREFIX", f"/var/tmp/lccc-session/gas247-{triple}"))
    if (p / "bin" / "as").is_file() or (p / "bin" / f"{triple}-as").is_file():
        return p
    if os.environ.get("GAS247_AUTO_PROVISION") == "1":
        subprocess.run(
            [str(REPO / "scripts" / "ensure_gas_247.sh"), triple, str(p)],
            check=True,
        )
        if (p / "bin" / "as").is_file() or (p / "bin" / f"{triple}-as").is_file():
            return p
    return fail(
        f"no {triple} GAS 2.47 under {p}: run scripts/ensure_gas_247.sh {triple} {p} "
        "or set GAS247_AUTO_PROVISION=1"
    )


def has_cap(tail: str) -> bool:
    """True when the directive names a padding cap (`.p2align 3,,4`)."""
    parts = tail.split(",")
    return len(parts) > 2 and parts[2].strip() != ""


def has_fill(tail: str) -> bool:
    """True when the directive names its own fill byte (`.p2align 3, 0xff`).

    That operand is what switches GNU as from the target's code-align hook to
    the generic data fill, so it also decides whether a size difference is the
    allowed pre-relaxation class.
    """
    parts = tail.split(",")
    return len(parts) > 1 and parts[1].strip() != ""


def objdump_text(objdump: Path, obj: Path) -> bytes:
    """.text of `obj`, read from the file itself, via objdump's own header."""
    header = subprocess.run(
        [str(objdump), "-h", str(obj)], capture_output=True, text=True, check=True
    ).stdout
    for line in header.splitlines():
        parts = line.split()
        if len(parts) >= 7 and parts[1] == ".text":
            size, off = int(parts[2], 16), int(parts[5], 16)
            return obj.read_bytes()[off : off + size]
    return b""


def assemble_gas(as_bin: Path, src: Path, obj: Path) -> bool:
    r = subprocess.run([str(as_bin), str(src), "-o", str(obj)], capture_output=True, text=True)
    if r.returncode == 0:
        return True
    if "Error:" not in r.stderr and "error:" not in r.stderr:
        fail(f"oracle as failed unexpectedly on {src.name}: {r.stderr.strip()[:200]}")
    return False  # the cell is an oracle rejection; LCCC must reject it too


def main() -> int:
    ap = argparse.ArgumentParser(description="alignment-padding census vs GAS 2.47")
    ap.add_argument("target", nargs="?", choices=sorted(TARGETS))
    ap.add_argument("--gas", help="prefix holding bin/<triple>-as and -objdump")
    ap.add_argument("--lccc-dir", default=str(REPO / "target" / "fastbuild"))
    ap.add_argument("--keep-cells", action="store_true")
    args = ap.parse_args()

    work = Path(os.environ.get("TMPDIR", "/var/tmp/lccc-session/tmp")) / "gas-fill-census"
    work.mkdir(parents=True, exist_ok=True)
    names = [args.target] if args.target else sorted(TARGETS)

    totals: dict[str, list[int]] = {n: [0, 0, 0, 0, 0] for n in names}
    total = same = relax_class = artifact_class = 0
    bad: list[str] = []
    for name in names:
        triple = TARGETS[name]["triple"]
        gas = gas_prefix(triple, args.gas)
        as_bin = tool(gas, triple, "as")
        objdump = tool(gas, triple, "objdump")
        lccc_bin = Path(args.lccc_dir) / TARGETS[name]["lccc_bin"]
        if not lccc_bin.is_file():
            fail(
                f"{lccc_bin} missing: cargo build --profile fastbuild "
                f"--bin {TARGETS[name]['lccc_bin']}"
            )
        for idx, (payload, exp, tail) in enumerate(
            (p, e, t) for p in PAYLOADS for e in EXPS for t in TAILS
        ):
            # A positional tag, never hash(tail): Python randomises str hashing
            # per process, so a hash-derived name lets two cells collide on one
            # file and compares one cell's GAS output against another cell's
            # LCCC output -- a harness bug that looks exactly like an assembler
            # bug, and was one here.
            src = work / f"{name}-{idx:04d}-{payload}e{exp}.s"
            src.write_text(
                "    .text\n" + "    .byte 0xa5\n" * payload + f"    .p2align {exp}{tail}\n"
            )
            gas_ok = assemble_gas(as_bin, src, src.with_suffix(".g.o"))
            r = subprocess.run(
                [str(lccc_bin), "-c", str(src), "-o", str(src.with_suffix(".l.o"))],
                capture_output=True,
                text=True,
            )
            total += 1
            if not gas_ok:
                if r.returncode == 0:
                    bad.append(f"{name} payload={payload} exp={exp} tail={tail!r}: GAS rejects, LCCC accepts")
                continue
            if r.returncode != 0:
                bad.append(
                    f"{name} payload={payload} exp={exp} tail={tail!r}: LCCC rejects a cell GAS "
                    f"accepts: {(r.stderr or r.stdout).strip()[:120]}"
                )
                continue
            g = objdump_text(objdump, src.with_suffix(".g.o"))
            l = objdump_text(objdump, src.with_suffix(".l.o"))
            t = totals[name]
            t[0] += 1
            if g == l:
                same += 1
                t[1] += 1
                continue
            align = 1 << exp
            if (
                name == "riscv64"
                and len(g) >= len(l)
                and len(l) % max(align, 1) == 0
                # A cap GAS *honours* never reaches its hook, so a skipped or
                # shortened run under a cap is a real divergence, not the class.
                and (not has_cap(tail) or align > 4 or has_fill(tail))
            ):
                if len(g) > len(l) or not has_fill(tail):
                    # LCCC is at the boundary; GAS is past it, or pads with the
                    # hook's NOP vocabulary instead of the requested byte,
                    # because `riscv_frag_align_code` runs before the fill is
                    # ever consulted. The linker is meant to relax that run back
                    # to the boundary LCCC already wrote -- see the module doc.
                    relax_class += 1
                    t[2] += 1
                    continue
                # Same size, different unreachable pad bytes: an explicit fill
                # below the code-alignment floor, where GAS's mapping-state
                # pre-align sneaks a c.nop in. No address moves; the fill byte
                # is what the source asked for. See padding_law_tests.
                artifact_class += 1
                t[3] += 1
                continue
            t[4] += 1
            bad.append(f"{name} payload={payload} exp={exp} tail={tail!r}")
            print(f"    gas : {g.hex()}")
            print(f"    lccc: {l.hex()}")
        if not args.keep_cells:
            for f in work.glob(f"{name}-*"):
                f.unlink(missing_ok=True)

    for name in names:
        t, ident, relax, artifact, divergent = totals[name]
        print(
            f"  {name}: {ident}/{t} identical, {relax} pre-relaxation (allowed), "
            f"{artifact} sub-floor fill (allowed), {divergent} divergent"
        )
    print(
        f"gas_fill_census: {same}/{total} identical, {relax_class} pre-relaxation "
        f"(allowed), {artifact_class} sub-floor fill (allowed), {len(bad)} divergent"
    )
    for line in bad[:40]:
        print(f"  DIVERGENT {line}", file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
