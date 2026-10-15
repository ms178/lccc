#!/usr/bin/env python3
"""RV64 shared address/preemption contract: independent ELF/loader algebra.

Uses the structurally checked RELA fixtures from the rejection suite. --oracle
runs the same semantic expectations against a real linker, not byte-identical
layout. Tool versions and every result are retained in the optional JSON.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import struct
import subprocess
import tempfile

from check_riscv_relocation_contract import fixture, sections, run

MASK = (1 << 64) - 1


def dynamic_symbols(sec):
    if ".dynsym" not in sec:
        return {}
    data = sec[".dynsym"][1]
    strings = sec[".dynstr"][1]
    result = {}
    for i in range(0, len(data), 24):
        name, info, other, shndx, value, size = struct.unpack_from("<IBBHQQ", data, i)
        end = strings.find(b"\0", name)
        assert end >= name
        result[strings[name:end].decode()] = dict(index=i // 24, binding=info >> 4,
                visibility=other & 3, shndx=shndx, value=value, size=size)
    return result


def verify(out, options, *, merged_visibility=None, definition_offset=0, places=1, data_offset=0):
    sec = sections(out)
    symbols = dynamic_symbols(sec)
    vis = options["visibility"] if merged_visibility is None else merged_visibility
    defined, binding = options["defined"], options["binding"]
    preemptible = binding != 0 and vis == 0
    absolute = options["absolute"] or (not defined and not preemptible)
    value = options["target_value"] if options["absolute"] else (
            sec[".text"][0][3] + definition_offset if defined else 0)
    addend = options["addend"]
    kind = options["kind"]
    exported = binding != 0 and vis in (0, 3) and (defined or preemptible)
    # A non-preemptible undefined weak may remain as an undefined PROTECTED
    # dynsym entry. It exports no definition; local resolution must still be 0.
    optional_undefined = binding == 2 and vis == 3 and not defined
    if not optional_undefined:
        assert ("target" in symbols) == exported, ("export", options, symbols)
    if "target" in symbols:
        symbol = symbols["target"]
        assert symbol["visibility"] == vis and symbol["binding"] == binding, symbol
        if options["absolute"]:
            assert symbol["shndx"] == 0xfff1, symbol
        elif not defined:
            assert symbol["shndx"] == 0 and symbol["value"] == 0, symbol
    relas = sec.get(".rela.dyn", (None, b""))[1]
    entries = [struct.unpack_from("<QQq", relas, i) for i in range(0, len(relas), 24)]
    if kind == 20:
        assert places == 1 and addend == 0
        # The canonical GOT_HI20 form has zero addend. Its HI20 must address
        # the actual GOT slot, not merely cause a plausible relocation table.
        got, got_data = sec[".got"]
        # Linkers may reserve a leading GOT word for _DYNAMIC. Identify the
        # one requested cell from relocation semantics (or its absolute value),
        # not from a hard-coded slot number shared with our implementation.
        if absolute and not preemptible:
            candidates = [got[3] + i for i in range(0, len(got_data), 8)
                          if struct.unpack_from("<Q", got_data, i)[0] == value]
        else:
            candidates = [p for p, _, _ in entries if got[3] <= p < got[3] + len(got_data)]
        assert len(candidates) == 1, (candidates, entries)
        target_place = candidates[0]
        pc = sec[".data"][0][3] + data_offset
        insn = struct.unpack_from("<I", sec[".data"][1], data_offset)[0]
        expected_hi = ((target_place - pc + 0x800) >> 12) & 0xfffff
        assert insn >> 12 == expected_hi, (insn, expected_hi)
        locations = [(target_place, struct.unpack_from("<Q", got_data, target_place - got[3])[0])]
    else:
        sh, data = sec[".data"]
        locations = [(sh[3] + i * 16, struct.unpack_from("<Q", data, i * 16)[0])
                     for i in range(places)]
    for place, initial in locations:
        at = [(info, a) for p, info, a in entries if p == place]
        if preemptible:
            assert at == [((symbols["target"]["index"] << 32) | 2, addend)], (at, options)
            # Simulate two loader resolutions. A RELATIVE entry cannot satisfy
            # these because changing the provider must change the address.
            for override in (0, 0x12345678):
                loaded = (override + at[0][1]) & MASK
                assert loaded == (override + addend) & MASK
        elif absolute:
            assert not at and initial == (value + addend) & MASK, (at, initial, value, addend)
        else:
            assert len(at) == 1 and at[0][0] == 3, (at, options)
            assert at[0][1] & MASK == (value + addend) & MASK, (at, value, addend)
            for base in (0x100000, 0x700000000):
                assert (base + at[0][1]) & MASK == (base + value + addend) & MASK


def cases():
    for binding, visibility, defined in [(0, 0, True)] + [
            (b, v, d) for b in (1, 2) for v in range(4) for d in (True, False)]:
        for absolute, target_value in [(False, 0)] + ([(True, 0), (True, 256)] if defined else []):
            for kind in (2, 20):
                for addend in ((-7, 0, 11, -(1 << 63), (1 << 63) - 1) if kind == 2 else (0,)):
                    options = dict(binding=binding, visibility=visibility, defined=defined,
                                   absolute=absolute, target_value=target_value, kind=kind, addend=addend)
                    accept = not (binding == 1 and visibility != 0 and not defined)
                    yield f"b{binding}-v{visibility}-d{defined}-abs{absolute}-{target_value}-k{kind}-a{addend}", options, accept


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc", required=True)
    ap.add_argument("--oracle")
    ap.add_argument("--json", type=Path)
    args = ap.parse_args()
    if not __debug__:
        ap.error("assertions are required; do not run this correctness gate with python -O")
    records, versions = [], {}
    try:
        measure(args, records, versions)
    except (AssertionError, OSError, subprocess.TimeoutExpired, ValueError, struct.error, KeyError, IndexError) as error:
        records.append(dict(case="infrastructure", tool="harness", passed=False, error=str(error)))
    summary = dict(passed=sum(r["passed"] for r in records), failed=sum(not r["passed"] for r in records),
                   skipped=0, versions=versions, records=records)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(summary, indent=2) + "\n")
    for record in records:
        if not record["passed"]:
            print("FAIL", record)
    print(f"RISC-V shared binding: {summary['passed']} PASS / {summary['failed']} FAIL / 0 SKIP")
    return int(bool(summary["failed"]))


def measure(args, records, versions):
    tools = {"lccc": [args.lccc, "-shared", "-nostdlib"]}
    if args.oracle:
        tools["oracle"] = [args.oracle, "-m", "elf64lriscv", "-shared", "--no-relax"]
    for name, command in tools.items():
        rc, banner = run([command[0], "--version"])
        assert rc == 0 and banner.strip(), f"{name} version probe failed: {rc}: {banner}"
        versions[name] = banner
    with tempfile.TemporaryDirectory(prefix="lccc-rv-binding-") as tmp:
        root = Path(tmp)
        for name, options, accept in cases():
            src = root / (name + ".o")
            src.write_bytes(fixture(**options))
            for tool, command in tools.items():
                out = root / f"{name}-{tool}.so"
                try:
                    rc, diagnostic = run(command + [str(src), "-o", str(out)])
                    assert (rc == 0) == accept, (rc, diagnostic)
                    if accept:
                        verify(out, options)
                    elif tool == "lccc":
                        assert "undefined non-default-visible" in diagnostic, diagnostic
                    records.append(dict(case=name, tool=tool, passed=True))
                except (AssertionError, OSError, subprocess.TimeoutExpired, ValueError,
                        struct.error, KeyError, IndexError) as error:
                    records.append(dict(case=name, tool=tool, passed=False, error=str(error)))
        # A reference in another TU constrains the definition, in both orders.
        # Each .data contribution carries an R_RISCV_64 to the same target.
        for vis in (1, 2, 3):
            for reverse in (False, True):
                options = dict(binding=1, visibility=0, defined=True, absolute=False,
                               target_value=0, kind=2, addend=11)
                definition = root / "definition.o"
                declaration = root / "declaration.o"
                definition.write_bytes(fixture(**options))
                declaration.write_bytes(fixture(**dict(options, visibility=vis, defined=False)).replace(b"_start\0", b"_extra\0"))
                inputs = [declaration, definition] if reverse else [definition, declaration]
                for tool, command in tools.items():
                    name = f"merged-v{vis}-reverse{reverse}"
                    out = root / (name + tool + ".so")
                    try:
                        rc, diagnostic = run(command + list(map(str, inputs)) + ["-o", str(out)])
                        assert rc == 0, diagnostic
                        verify(out, options, merged_visibility=vis, definition_offset=12 if reverse else 0)
                        sec = sections(out)
                        entries = [struct.unpack_from("<QQq", sec[".rela.dyn"][1], i)
                                   for i in range(0, len(sec[".rela.dyn"][1]), 24)]
                        expected = sec[".text"][0][3] + (12 if reverse else 0) + 11
                        assert len(entries) == 2 and len({p for p, _, _ in entries}) == 2, entries
                        lo = sec[".data"][0][3]
                        hi = lo + len(sec[".data"][1])
                        assert all(lo <= p <= hi - 8 and info == 3 and a == expected
                                   for p, info, a in entries), entries
                        records.append(dict(case=name, tool=tool, passed=True))
                    except (AssertionError, OSError, subprocess.TimeoutExpired, ValueError,
                            struct.error, KeyError, IndexError) as error:
                        records.append(dict(case=name, tool=tool, passed=False, error=str(error)))

        # Resolution strength and section/ABS classification must not depend
        # on which TU was scanned first. Every case has two pointer uses.
        for vis in (0, 1, 2, 3):
            for reverse in (False, True):
                options = dict(binding=1, visibility=vis, defined=False, absolute=False,
                               target_value=0, kind=2, addend=11)
                weak = root / "weak.o"
                strong = root / "strong.o"
                weak.write_bytes(fixture(**dict(options, binding=2)).replace(b"_start\0", b"_extra\0"))
                strong.write_bytes(fixture(**options))
                inputs = [strong, weak] if reverse else [weak, strong]
                check_merged(tools, records, root, f"undefined-strength-v{vis}-reverse{reverse}",
                             inputs, options, accept=vis == 0)
        for vis in (0, 2, 3):
            for reverse in (False, True):
                for strong_abs in (False, True):
                    options = dict(binding=1, visibility=vis, defined=True, absolute=strong_abs,
                                   target_value=256 if strong_abs else 0, kind=2, addend=11)
                    strong = root / "strong.o"
                    weak = root / "weak.o"
                    strong.write_bytes(fixture(**options))
                    weak.write_bytes(fixture(**dict(options, binding=2, absolute=not strong_abs,
                        target_value=0 if strong_abs else 256)).replace(b"_start\0", b"_extra\0"))
                    inputs = [strong, weak] if reverse else [weak, strong]
                    check_merged(tools, records, root,
                        f"definition-strength-abs{strong_abs}-v{vis}-reverse{reverse}", inputs,
                        options, definition_offset=0 if reverse else 12)
        for reverse in (False, True):
            local = root / "local.o"
            collision = root / "collision.o"
            options = dict(binding=0, visibility=0, defined=True, absolute=False,
                           target_value=0, kind=20, addend=0)
            local.write_bytes(fixture(**options, payload=struct.pack("<I", 0x17) + bytes(12)))
            collision.write_bytes(fixture(kind=0, binding=1,
                target_name=f"__local_{int(reverse)}_target_0").replace(b"_start\0", b"_extra\0"))
            inputs = [collision, local] if reverse else [local, collision]
            check_merged(tools, records, root, f"got-key-collision-reverse{reverse}",
                         inputs, options, definition_offset=12 if reverse else 0,
                         local_data_offset=32 if reverse else 0)

        for symbol_name in ("", "target"):
            src = root / "duplicate-local.o"
            src.write_bytes(fixture(kind=20, duplicate_local=True, target_name=symbol_name,
                payload=struct.pack("<IIII", 0x17, 0x17, 0, 0)))
            for tool, command in tools.items():
                name = f"duplicate-local-name-{symbol_name or 'empty'}"
                out = root / (name + tool + ".so")
                try:
                    rc, diagnostic = run(command + [str(src), "-o", str(out)])
                    assert rc == 0, diagnostic
                    sec = sections(out)
                    entries = [struct.unpack_from("<QQq", sec[".rela.dyn"][1], i)
                               for i in range(0, len(sec[".rela.dyn"][1]), 24)]
                    assert len(entries) == 2 and len({p for p, _, _ in entries}) == 2, entries
                    for offset in (0, 4):
                        value = sec[".text"][0][3] + offset
                        slots = [p for p, info, a in entries if info == 3 and a == value]
                        assert len(slots) == 1, (value, entries)
                        got, _ = sec[".got"]
                        assert got[3] <= slots[0] <= got[3] + got[5] - 8, slots
                        pc = sec[".data"][0][3] + offset
                        instruction = struct.unpack_from("<I", sec[".data"][1], offset)[0]
                        assert instruction & 0xfff == 0x17, instruction
                        assert instruction >> 12 == ((slots[0] - pc + 0x800) >> 12) & 0xfffff
                    records.append(dict(case=name, tool=tool, passed=True))
                except (AssertionError, OSError, subprocess.TimeoutExpired, ValueError,
                        struct.error, KeyError, IndexError) as error:
                    records.append(dict(case=name, tool=tool, passed=False, error=str(error)))


def check_merged(tools, records, root, name, inputs, options, *, accept=True,
                 definition_offset=0, local_data_offset=0):
    for tool, command in tools.items():
        out = root / (name + tool + ".so")
        try:
            rc, diagnostic = run(command + list(map(str, inputs)) + ["-o", str(out)])
            assert (rc == 0) == accept, (rc, diagnostic)
            if accept:
                if local_data_offset:
                    # Only the second input has a GOT instruction. Derive its
                    # position from the executable AUIPC, not output section
                    # concatenation (LLD and LCCC group .data.more differently).
                    sec = sections(out)
                    data = sec[".data"][1]
                    sites = [i for i in range(0, len(data) - 3, 4)
                             if struct.unpack_from("<I", data, i)[0] & 0x7f == 0x17]
                    assert len(sites) == 1, sites
                    data_offset = sites[0]
                else:
                    data_offset = 0
                verify(out, options, definition_offset=definition_offset, data_offset=data_offset)
            elif tool == "lccc":
                assert "undefined non-default-visible" in diagnostic, diagnostic
            records.append(dict(case=name, tool=tool, passed=True))
        except (AssertionError, OSError, subprocess.TimeoutExpired, ValueError,
                struct.error, KeyError, IndexError) as error:
            records.append(dict(case=name, tool=tool, passed=False, error=str(error)))


if __name__ == "__main__":
    raise SystemExit(main())
