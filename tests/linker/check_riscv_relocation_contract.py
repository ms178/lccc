#!/usr/bin/env python3
"""A14-17: reject lost/invalid relocations without rejecting valid RV64 ELF.

Fixtures are deterministic ELF64 RELA objects (no assembler/toolchain dependency).
--oracle additionally checks acceptance against a real RISC-V-capable ELF linker.
Every invocation has a private temporary directory and a bounded timeout; a
panic, signal, missing tool or malformed output is never a valid rejection.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import struct
import sys
import subprocess
import tempfile


def fixture(kind=2, offset=0, symidx=2, addend=0, binding=0, visibility=0,
            defined=True, payload=None, uleb=None, no_symbols=False):
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
    strings = b"\0_start\0target\0"
    # exit(0), independent of the data-relocation tests.
    text = struct.pack("<III", 0x00000513, 0x05D00893, 0x00000073)
    if uleb is not None:
        text += bytes(0x1010 - len(text))
    symbols = bytes(24)
    symbols += struct.pack("<IBBHQQ", 8, binding << 4, visibility,
                           1 if defined else 0, 0, 0)
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
        rela = b"".join(struct.pack("<QQq", offset, ((1 if k == 60 else 2) << 32) | k, addend)
                        for k in sequence)
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
                            shoff, 0, 64, 0, 0, 64, len(headers), 7)
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
            src.write_bytes(fixture(**options))
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
                                            command=command, returncode=rc, diagnostic=diagnostic))
                    except (AssertionError, OSError, subprocess.TimeoutExpired, ValueError,
                            struct.error, KeyError) as error:
                        records.append(dict(case=name, shared=shared, tool=tool, passed=False,
                                            error=str(error), command=command))
    summary = dict(oracle_version=(run([args.oracle, "--version"])[1].strip() if args.oracle else None),
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
