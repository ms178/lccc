#!/usr/bin/env python3
"""A14-17: reject lost/invalid relocations without rejecting valid RV64 ELF.

Fixtures are deterministic ELF64 RELA objects (no assembler/toolchain dependency).
--oracle additionally checks acceptance against a real RISC-V-capable ELF linker.
Every invocation has a private temporary directory and a bounded timeout; a
panic, signal, missing tool or malformed output is never a valid rejection.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import random
from pathlib import Path
import struct
import sys
import subprocess
import tempfile


def fixture(kind=2, offset=0, symidx=2, addend=0, binding=0, visibility=0,
            defined=True, payload=None, uleb=None, no_symbols=False, displacement=None,
            absolute=False, target_value=0, target_name="target", duplicate_local=False):
    """NULL, .text, .data, .data.more, .rela.data, .symtab, .strtab, .shstrtab.

    .data.more deliberately follows .data after section merging, so checking
    the merged buffer alone cannot catch a relocation spilling across inputs.
    """
    names = ["", ".text", ".data", ".data.more", ".rela.data", ".symtab",
             ".strtab", ".shstrtab"]
    shstr = b"\0"
    name_offsets = [0]
    for name in names[1:]:
        name_offsets.append(len(shstr))
        shstr += name.encode() + b"\0"
    strings = b"\0_start\0" + target_name.encode() + b"\0"
    # exit(0), independent of the data-relocation tests.
    text = struct.pack("<III", 0x00000513, 0x05D00893, 0x00000073)
    if uleb is not None:
        text += bytes(0x1010 - len(text))
    target_section = 0xfff1 if absolute else 1
    if displacement is not None:
        assert not absolute, "a displacement fixture cannot also be absolute"
        target_section, target_value = 2, offset + displacement
    symbols = bytes(24)
    symbols += struct.pack("<IBBHQQ", 8, binding << 4, visibility,
                           target_section if defined else 0, target_value, 0)
    symbols += struct.pack("<IBBHQQ", 1, 0x12, 0, 1, 0, len(text))
    # Local symbols precede globals. Relocations normally target symbol 1;
    # symidx=2 denotes the abstract target in callers, not an ELF index.
    target_idx = 1
    if binding:
        symbols = bytes(24) + symbols[48:72] + symbols[24:48]
        target_idx = 2
    actual_idx = target_idx if symidx == 2 else symidx
    rela = struct.pack("<QQq", offset, (actual_idx << 32) | kind, addend)
    local_count = 1 if binding else 2
    if uleb is not None:
        set_value, sub_value, sequence = uleb
        # Two local labels and a global entry, with no intermediate mutation.
        symbols = bytes(24) + struct.pack("<IBBHQQ", 8, 0, 0, 1, set_value, 0)
        symbols += struct.pack("<IBBHQQ", 8, 0, 0, 1, sub_value, 0)
        symbols += struct.pack("<IBBHQQ", 1, 0x12, 0, 1, 0, 12)
        local_count = 3
        rela = b""
        for operation in sequence:
            k, delta = (operation, 0) if isinstance(operation, int) else operation
            rela += struct.pack("<QQq", offset + delta, ((1 if k == 60 else 2) << 32) | k, addend)
    if duplicate_local:
        assert kind == 20 and binding == 0 and defined and not absolute
        # Two legitimate local symbols may have equal (or empty) names in a
        # single object, while denoting distinct locations in its .text.
        symbols = bytes(24)
        for value in (0, 4):
            symbols += struct.pack("<IBBHQQ", 8, 0, 0, 1, value, 0)
        local_count = 3
        rela = b"".join(struct.pack("<QQq", i * 4, ((i + 1) << 32) | 20, 0)
                        for i in range(2))
    if no_symbols:
        symbols = b""
    chunks = [b"", text, bytes(16) if payload is None else payload,
              b"\xa5" * 16, rela, symbols, strings, shstr]
    meta = [(0, 0, 0, 0, 0, 0), (1, 6, 0, 0, 4, 0), (1, 3, 0, 0, 8, 0),
            (1, 3, 0, 0, 8, 0), (4, 0, 5, 2, 8, 24),
            (0 if no_symbols else 2, 0, 6, local_count, 8, 24), (3, 0, 0, 0, 1, 0),
            (3, 0, 0, 0, 1, 0)]
    data = bytearray(64)
    headers = []
    for i, (chunk, (ty, flags, link, info, align, entsize)) in enumerate(zip(chunks, meta)):
        if i == 0:
            headers.append(bytes(64))
            continue
        data.extend(bytes((-len(data)) % align))
        start = len(data)
        data.extend(chunk)
        headers.append(struct.pack("<IIQQQQIIQQ", name_offsets[i], ty, flags,
                                   0, start, len(chunk), link, info, align, entsize))
    data.extend(bytes((-len(data)) % 8))
    shoff = len(data)
    data.extend(b"".join(headers))
    ident = b"\x7fELF\x02\x01\x01" + bytes(9)
    data[:64] = struct.pack("<16sHHIQQQIHHHHHH", ident, 1, 243, 1, 0, 0,
                            shoff, int(kind in (44, 45)), 64, 0, 0, 64, len(headers), 7)
    return bytes(data)


def sections(path):
    data = path.read_bytes()
    hdr = struct.unpack_from("<16sHHIQQQIHHHHHH", data)
    assert hdr[0][:7] == b"\x7fELF\x02\x01\x01" and hdr[2] == 243, "not RV64 ELF"
    assert hdr[11] == 64, "unexpected section header size"
    headers = [struct.unpack_from("<IIQQQQIIQQ", data, hdr[6] + i * 64)
               for i in range(hdr[12])]
    names = headers[hdr[13]]
    strings = data[names[4]:names[4] + names[5]]
    result = {}
    for sh in headers:
        end = strings.find(b"\0", sh[0])
        assert end >= sh[0], "unterminated section name"
        name = strings[sh[0]:end].decode()
        result[name] = (sh, data[sh[4]:sh[4] + sh[5]])
    return result


def run(command):
    p = subprocess.run(command, capture_output=True, text=True, timeout=20)
    diagnostic = p.stdout + p.stderr
    if p.returncode < 0 or any(s in diagnostic.lower() for s in
                              ("panicked at", "internal compiler error", "ccc: internal error", "segmentation fault")):
        raise AssertionError(f"crash is not a diagnostic: {command}: {diagnostic}")
    return p.returncode, diagnostic


def decode_displacement(kind, data):
    """Independent ISA immediate decoder, not the linker's patch implementation."""
    word = int.from_bytes(data, "little")
    if kind == 16:
        value = ((word >> 31) << 12) | (((word >> 7) & 1) << 11)
        value |= ((word >> 25) & 63) << 5 | ((word >> 8) & 15) << 1
        bits, mask = 13, 0xfe000f80
    elif kind == 17:
        value = ((word >> 31) << 20) | (((word >> 12) & 255) << 12)
        value |= ((word >> 20) & 1) << 11 | ((word >> 21) & 1023) << 1
        bits, mask = 21, 0xfffff000
    elif kind == 44:
        value = ((word >> 12) & 1) << 8 | ((word >> 10) & 3) << 3
        value |= ((word >> 5) & 3) << 6 | ((word >> 3) & 3) << 1 | ((word >> 2) & 1) << 5
        bits, mask = 9, 0x1c7c
    else:
        assert kind == 45, kind
        value = ((word >> 12) & 1) << 11 | ((word >> 11) & 1) << 4
        value |= ((word >> 9) & 3) << 8 | ((word >> 8) & 1) << 10
        value |= ((word >> 7) & 1) << 6 | ((word >> 6) & 1) << 7
        value |= ((word >> 3) & 7) << 1 | ((word >> 2) & 1) << 5
        bits, mask = 12, 0x1ffc
    signed = value - (1 << bits) if value & (1 << (bits - 1)) else value
    return signed, word & ~mask


def cases():
    # Unknown numbers span unsigned r_type, for local, preemptible, hidden,
    # protected, weak and undefined symbols, not just the original 255 case.
    for kind in (255, 0x7fffffff, 0xffffffff):
        for binding, visibility, defined in ((0, 0, True), (1, 0, True),
                (1, 2, True), (1, 3, True), (2, 0, True), (1, 0, False)):
            yield (f"unknown-{kind}-{binding}-{visibility}-{defined}",
                   dict(kind=kind, binding=binding, visibility=visibility, defined=defined),
                   False, "unsupported RISC-V relocation")
    for index in (3, 0xffffffff):
        for kind in (2, 19, 20, 21, 22):
            yield f"bad-symbol-{kind}-{index}", dict(kind=kind, symidx=index), False, "symbol index"
    # Fixed-width stores, arithmetic, compressed and paired call instructions.
    for kind, width in ((2, 8), (1, 4), (33, 1), (34, 2), (35, 4), (36, 8),
                        (37, 1), (38, 2), (39, 4), (40, 8), (44, 2), (45, 2),
                        (53, 1), (54, 1), (55, 2), (56, 4), (19, 8), (22, 4)):
        for offset in sorted({17 - width, 16, (1 << 64) - 1}):
            yield f"bad-extent-{kind}-{offset}", dict(kind=kind, offset=offset), False, "exceeds input section"
    for addend in (-7, 0, 11):
        for offset in (0, 8):
            yield f"valid-64-{offset}-{addend}", dict(offset=offset, addend=addend), True, ""
    # Boundary, alignment and seeded interior points for all four branch forms.
    # Both site and symbol stay inside this input contribution, including every
    # out-of-range control: these are not invalid-section/extent substitutes.
    for kind, bits, width, opcode in ((16, 13, 4, 0x63), (17, 21, 4, 0xef),
                                     (44, 9, 2, 0xc001), (45, 12, 2, 0xa001)):
        limit = 1 << (bits - 1)
        site = limit + 4
        payload = bytearray(2 * site + 8)
        payload[site:site + width] = opcode.to_bytes(width, "little")
        valid = {-limit, -limit + 2, -2, 0, 2, limit - 4, limit - 2, 4}
        rng = random.Random(kind)
        while len(valid) < 24:
            valid.add(-limit + 2 * rng.randrange(limit))
        for delta in sorted(valid):
            yield (f"valid-range-{kind}-{delta}", dict(kind=kind, offset=site,
                   payload=bytes(payload), displacement=delta), True, "")
        for delta in (-limit - 4, -limit - 2, limit, limit + 2, -limit + 1, -1, 1, limit - 1):
            diagnostic = "misaligned" if delta & 1 else "does not fit"
            yield (f"bad-range-{kind}-{delta}", dict(kind=kind, offset=site,
                   payload=bytes(payload), displacement=delta), False, diagnostic)
    # NONE and RELAX must not be confused with unknown relocation kinds.
    for kind in (0, 51):
        yield f"valid-marker-{kind}", dict(kind=kind, symidx=0), True, ""

    for width in range(1, 11):
        payload = bytes(16 - width) + bytes([0x80] * (width - 1) + [0])
        yield (f"valid-uleb-padded-{width}", dict(offset=16 - width, payload=payload,
               uleb=(0x1000, 0xfff, (60, 61))), True, "")
    yield ("valid-none-empty", dict(kind=0, symidx=0,
           offset=0xffffffffffffffff, payload=b""), True, "")
    yield ("bad-uleb-unterminated", dict(offset=15, payload=bytes(15) + b"\x80",
           uleb=(0x1000, 0xfff, (60, 61))), False, "ULEB128")
    yield ("bad-uleb-capacity", dict(offset=15, payload=bytes(16),
           uleb=(0x1000, 0, (60, 61))), False, "ULEB128")
    for label, sequence in (("set-only", (60,)), ("sub-only", (61,)),
                            ("reversed", (61, 60)), ("different-offset", (60, (61, 1)))):
        yield (f"bad-uleb-{label}", dict(uleb=(0x1000, 0xfff, sequence)), False, "unpaired")
    yield ("bad-uleb-underflow", dict(uleb=(0xfff, 0x1000, (60, 61))), False, "ULEB128")
    yield ("bad-uleb-oversized", dict(offset=6, payload=bytes(6) + b"\x80" * 9 + b"\x02",
           uleb=(0x1000, 0xfff, (60, 61))), False, "ULEB128")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc", help="lccc-riscv driver")
    ap.add_argument("--oracle-only", action="store_true", help="validate fixtures with the real linker only")
    ap.add_argument("--oracle", help="optional real linker, e.g. ld.lld")
    ap.add_argument("--json", type=Path)
    ap.add_argument("--only", help="comma-separated case prefixes; never an empty selection")
    args = ap.parse_args()
    if not __debug__:
        ap.error("optimized Python disables test assertions")
    if args.oracle_only and not args.oracle:
        ap.error("--oracle-only requires --oracle")
    if not args.oracle_only and not args.lccc:
        ap.error("--lccc is required unless --oracle-only")
    tools = ["oracle"] if args.oracle_only else (["lccc", "oracle"] if args.oracle else ["lccc"])
    selected = [case for case in cases() if not args.only or
                any(case[0].startswith(prefix) for prefix in args.only.split(",") if prefix)]
    if not selected:
        ap.error("empty fixture selection")
    records = []
    with tempfile.TemporaryDirectory(prefix="lccc-rv-reloc-") as tmp:
        root = Path(tmp)
        for name, options, accept, expected in selected:
            src = root / (name + ".o")
            fixture_data = fixture(**options)
            src.write_bytes(fixture_data)
            fixture_hash = hashlib.sha256(fixture_data).hexdigest()
            # Check the fixture's own structure; oracle runs also validate it.
            assert sections(src)[".rela.data"][0][5] % 24 == 0
            for shared in (False, True):
                for tool in tools:
                    out = root / (name + f"-{shared}-{tool}.elf")
                    command = ([args.lccc, "-nostdlib", "-shared" if shared else "-static"]
                               if tool == "lccc" else
                               [args.oracle, "-m", "elf64lriscv", "--no-relax"] +
                               (["-shared"] if shared else []))
                    command += [str(src), "-o", str(out)]
                    rc, diagnostic = None, ""
                    try:
                        rc, diagnostic = run(command)
                        assert (rc == 0) == accept, f"expected accept={accept}, exit={rc}: {diagnostic}"
                        if not accept and tool == "lccc":
                            # ET_EXEC reports an undefined symbol before dispatch;
                            # the ET_DYN twin must reach unknown-type rejection.
                            needle = ("undefined symbols" if not shared and
                                      not options.get("defined", True) else expected)
                            accepted = [needle]
                            if needle == "exceeds input section":
                                # The shared ELF reader rejects an out-of-range
                                # starting byte before target-width validation.
                                accepted.append("outside the section it patches")
                            assert any(n in diagnostic for n in accepted), f"wrong diagnostic: {diagnostic}"
                            assert not out.exists(), "published output despite rejection"
                        if accept:
                            sec = sections(out)
                            if options.get("uleb") is not None:
                                offset = options.get("offset", 0)
                                original = options["payload"][offset:]
                                width = next(i + 1 for i, b in enumerate(original) if b < 128)
                                result = options["uleb"][0] - options["uleb"][1]
                                expected_field = bytes(((result >> (7 * i)) & 127) | (128 if i + 1 < width else 0)
                                                       for i in range(width))
                                assert sec[".data"][1][offset:offset + width] == expected_field
                            elif options.get("displacement") is not None:
                                offset, kind = options["offset"], options["kind"]
                                width = 2 if kind in (44, 45) else 4
                                data = sec[".data"][1]
                                field = data[offset:offset + width]
                                decoded, preserved = decode_displacement(kind, field)
                                original = options["payload"]
                                _, original_bits = decode_displacement(kind, original[offset:offset + width])
                                assert decoded == options["displacement"], (decoded, options["displacement"])
                                assert preserved == original_bits, "opcode/register bits overwritten"
                                assert data[:offset] == original[:offset], "prefix overwritten"
                                assert data[offset + width:len(original)] == original[offset + width:], "suffix overwritten"
                            elif options.get("kind", 2) == 2:
                                offset = options.get("offset", 0)
                                value = sec[".text"][0][3] + options.get("addend", 0)
                                if shared:
                                    relas = sec[".rela.dyn"][1]
                                    entries = [struct.unpack_from("<QQq", relas, i)
                                               for i in range(0, len(relas), 24)]
                                    expected_rela = (sec[".data"][0][3] + offset, 3, value)
                                    assert expected_rela in entries, (expected_rela, entries)
                                else:
                                    assert struct.unpack_from("<Q", sec[".data"][1], offset)[0] == value
                            # No relocation may touch the adjacent contribution.
                            # lld may retain its name instead of merging .data.*.
                            adjacent = sec.get(".data.more", sec[".data"])[1][-16:]
                            assert adjacent == b"\xa5" * 16, "adjacent section overwritten"
                        records.append(dict(case=name, shared=shared, tool=tool, passed=True,
                                            command=command, returncode=rc, diagnostic=diagnostic,
                                            expected_accept=accept, fixture_sha256=fixture_hash))
                    except (AssertionError, OSError, subprocess.TimeoutExpired, ValueError,
                            struct.error, KeyError) as error:
                        records.append(dict(case=name, shared=shared, tool=tool, passed=False,
                                            error=str(error), command=command, returncode=rc,
                                            diagnostic=diagnostic, expected_accept=accept,
                                            fixture_sha256=fixture_hash))
    summary = dict(fixture_generator_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                   oracle_version=(run([args.oracle, "--version"])[1].strip() if args.oracle else None),
                   passed=sum(r["passed"] for r in records),
                   failed=sum(not r["passed"] for r in records), skipped=0, records=records)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(summary, indent=2) + "\n")
    for record in records:
        if not record["passed"]:
            print("FAIL", record)
    print(f"RISC-V relocation contract: {summary['passed']} PASS / {summary['failed']} FAIL / 0 SKIP")
    return int(bool(summary["failed"]))


if __name__ == "__main__":
    raise SystemExit(main())
