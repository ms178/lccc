#!/usr/bin/env python3
"""ADR/ADRP fields, operand classes, local relocations and all ELF output modes.

No external compiler oracle is silently substituted. Expected fields are decoded
independently from the architectural signed-21 representation. CLI tools must
succeed without diagnostics; malformed inputs must fail, not crash. QEMU is not
needed to prove the instruction address bits or ELF relocation contract.
"""
import os
from pathlib import Path
import struct
import subprocess
import tempfile

# Assertions are the gate verdicts, not optional debugging checks. Refuse
# -O/-OO and PYTHONOPTIMIZE before any compiler or runtime can be invoked.
if not __debug__:
    raise SystemExit("AArch64 contract checks require Python assertions; disable -O/-OO and PYTHONOPTIMIZE")

ROOT = Path(__file__).resolve().parents[2]
CC = os.environ.get("CCC_ARM", str(ROOT / "target/fastbuild/lccc-arm"))


def run(args, good=True):
    p = subprocess.run([CC, *map(str, args)], capture_output=True, text=True, timeout=30)
    if good:
        assert p.returncode == 0 and not p.stderr.strip(), (args, p.returncode, p.stderr)
    else:
        assert p.returncode > 0 and p.stderr.strip(), (args, p.returncode, p.stderr)
        assert not any(s in p.stderr.lower() for s in ("panicked at", "stack overflow", "internal compiler error")), p.stderr
    return p


def sections(path):
    data = path.read_bytes()
    assert data[:6] == b"\x7fELF\x02\x01", path
    off = struct.unpack_from("<Q", data, 40)[0]
    size, count, names = struct.unpack_from("<HHH", data, 58)
    assert size == 64
    rows = [struct.unpack_from("<IIQQQQIIQQ", data, off + i * size) for i in range(count)]
    strings = data[rows[names][4]:rows[names][4] + rows[names][5]]
    return data, {strings[r[0]:].split(b"\0", 1)[0].decode(): r for r in rows}


def mapped_marker(path, marker, skip=0):
    # Native ET_EXEC need not carry section headers. Locate deliberate unique
    # fixture bytes and map their file position through PT_LOAD instead.
    data = path.read_bytes()
    token = struct.pack("<Q", marker)
    assert data.count(token) == 1
    position = data.index(token) + skip
    phoff = struct.unpack_from("<Q", data, 32)[0]
    phsize, phnum = struct.unpack_from("<HH", data, 54)
    assert phsize == 56
    headers = [struct.unpack_from("<IIQQQQQQ", data, phoff + i * phsize) for i in range(phnum)]
    load = [h for h in headers if h[0] == 1 and h[2] <= position < h[2] + h[5]]
    assert len(load) == 1
    return data, position, load[0][3] + position - load[0][2], headers


def displacement(word):
    raw = ((word >> 29) & 3) | (((word >> 5) & 0x7ffff) << 2)
    return raw - (1 << 21) if raw & (1 << 20) else raw


def main():
    checks = 0
    with tempfile.TemporaryDirectory(prefix="lccc-arm-address-") as td:
        w = Path(td)
        src, obj, exe = w / "case.s", w / "case.o", w / "case.elf"
        values = [-1048576, -1048575, -4097, -4096, -4, -3, -2, -1, 0, 1, 2, 3, 4, 4095, 4096, 1048574, 1048575]
        registers = [f"x{i}" for i in range(31)] + ["xzr"]
        src.write_text(".text\n" + "\n".join(f"adr {reg}, #{n}" for reg in registers for n in values) + "\n")
        run(["-c", src, "-o", obj])
        data, rows = sections(obj)
        text = rows[".text"]
        for i, (reg, expected) in enumerate((r, n) for r in range(32) for n in values):
            word = struct.unpack_from("<I", data, text[4] + i * 4)[0]
            assert word & 0x9f00001f == 0x10000000 | reg
            assert displacement(word) == expected
            checks += 1
        bad = [f"adr x0, #{n}" for n in [-9223372036854775808, -1048577, 1048576, 9223372036854775807]]
        for mnemonic in ["adr", "adrp"]:
            bad += [f"{mnemonic} {reg}, target" for reg in ["w0", "w30", "wzr", "sp", "wsp", "d0", "q0", "v0.2d", "x31", "x32", "x00"]]
            bad += [mnemonic, f"{mnemonic} x0", f"{mnemonic} x0, target, x1"]
        for asm in bad:
            src.write_text(".text\n" + asm + "\n")
            run(["-c", src, "-o", obj], False)
            checks += 1

        # ADRP must not be folded against the section's temporary zero base;
        # even local same-section targets require relocation and their addend.
        src.write_text(".text\nlocal: nop\nadrp x0, local+4096\n")
        run(["-c", src, "-o", obj])
        data, rows = sections(obj)
        rela = rows[".rela.text"]
        offset, info, addend = struct.unpack_from("<QQq", data, rela[4])
        assert offset == 4 and info & 0xffffffff == 275 and addend == 4096
        checks += 1

        provider = w / "provider.s"
        provider.write_text(".text\n.global support\nsupport: ret\n")
        run(["-c", provider, "-o", w / "provider.o"])
        run(["-nostdlib", "-shared", w / "provider.o", "-o", w / "libprovider.so"])
        for mnemonic, opcode, kind in [("adr", 0x10000000, 274), ("adrp", 0x90000000, 275)]:
            for mode in ["static", "shared", "dynamic"]:
                src.write_text(f".text\n.quad 0x8c67d1284e9ab053\n.global _start\n_start: {mnemonic} x0, target\n"
                               + ("bl support\n" if mode == "dynamic" else "")
                               + "ret\n.data\n.local target\ntarget: .quad 0x52fd03498abc7e91\n")
                run(["-c", src, "-o", obj])
                flags = ["-shared"] if mode == "shared" else []
                libs = [w / "libprovider.so"] if mode == "dynamic" else []
                command = ["-nostdlib", *flags, obj, *libs, "-o", exe]
                run(command)
                linked, _, p, headers = mapped_marker(exe, 0x8c67d1284e9ab053, 8)
                _, _, s, _ = mapped_marker(exe, 0x52fd03498abc7e91)
                assert struct.unpack_from("<H", linked, 16)[0] == (3 if mode == "shared" else 2)
                if mode == "dynamic":
                    assert any(h[0] == 3 for h in headers), "dynamic emitter not exercised"
                original, inputs = sections(obj)
                rel = inputs[".rela.text"]
                assert struct.unpack_from("<Q", original, rel[4] + 8)[0] & 0xffffffff == kind
                for want in [-1048577, -1048576, -1, 0, 1, 1048575, 1048576]:
                    add = (p + want - s if mnemonic == "adr"
                           else (p & ~4095) + want * 4096 - s)
                    changed = bytearray(original)
                    struct.pack_into("<q", changed, rel[4] + 16, add)
                    obj.write_bytes(changed)
                    good = -1048576 <= want <= 1048575
                    run(command, good)
                    if good:
                        binary, site, _, _ = mapped_marker(exe, 0x8c67d1284e9ab053, 8)
                        word = struct.unpack_from("<I", binary, site)[0]
                        assert word & 0x9f00001f == opcode
                        assert displacement(word) == want, (mode, mnemonic, want, hex(word))
                    checks += 1
                # Bad input sites must not wrap into headers or patch a
                # neighboring section, even when the output file is large.
                for bad in [inputs[".text"][5] - 3, inputs[".text"][5], (1 << 64) - 1]:
                    changed = bytearray(original)
                    struct.pack_into("<Q", changed, rel[4], bad)
                    obj.write_bytes(changed)
                    run(command, False)
                    checks += 1
                changed = bytearray(original)
                struct.pack_into("<Q", changed, rel[4] + 8, (0xffffffff << 32) | kind)
                obj.write_bytes(changed)
                run(command, False)
                checks += 1
                # NONE does not patch the instruction or resolve a target.
                # Keep a structurally valid offset for the generic ELF parser.
                struct.pack_into("<Q", changed, rel[4], 0)
                struct.pack_into("<Q", changed, rel[4] + 8, 0)
                obj.write_bytes(changed)
                run(command)
                checks += 1
        # ELF symbol zero is LOCAL+UNDEF: numeric S=0 is not proof that
        # load bias cancels. Preserve a sentinel output on every rejected link.
        src.write_text(".text\n.quad 0x8c67d1284e9ab053\n.global entry\n"
                       "entry: adr x0,target\nret\n.data\n.local target\n"
                       "target: .quad 0x52fd03498abc7e91\n")
        run(["-c", src, "-o", obj])
        original, rows = sections(obj)
        rela_site = rows[".rela.text"][4]
        sym_index = struct.unpack_from("<Q", original, rela_site + 8)[0] >> 32
        sym_site = rows[".symtab"][4] + 24 * sym_index
        for case in ["zero", "undef", "abs", "missing-section"]:
            changed = bytearray(original)
            if case == "zero":
                struct.pack_into("<Q", changed, rela_site + 8, 274)
            else:
                shndx = {"undef": 0, "abs": 0xfff1, "missing-section": 0xff00}[case]
                struct.pack_into("<H", changed, sym_site + 6, shndx)
            obj.write_bytes(changed)
            exe.write_bytes(b"must not overwrite an existing output")
            run(["-nostdlib", "-shared", obj, "-o", exe], False)
            assert exe.read_bytes() == b"must not overwrite an existing output", case
            checks += 1
        # STT_SECTION and a named local are independent positive controls.
        for section_symbol in [False, True]:
            changed = bytearray(original)
            if section_symbol:
                changed[sym_site + 4] = 3  # STB_LOCAL | STT_SECTION
            obj.write_bytes(changed)
            run(["-nostdlib", "-shared", obj, "-o", exe])
            data, site, p, _ = mapped_marker(exe, 0x8c67d1284e9ab053, 8)
            _, _, target, _ = mapped_marker(exe, 0x52fd03498abc7e91)
            assert p + displacement(struct.unpack_from("<I", data, site)[0]) == target
            checks += 1
        # A real, nonallocated input section is not an address that relocates
        # with this DSO, even when its symbol is local and numerically in range.
        src.write_text('.text\n.global entry\nentry: adr x0,target\nret\n'
                       '.section .debug_target,"",%progbits\n.local target\ntarget: .quad 1\n')
        run(["-c", src, "-o", obj])
        p = run(["-nostdlib", "-shared", obj, "-o", exe], False)
        assert "local allocated section-relative" in p.stderr, p.stderr
        checks += 1
        # Same-name local symbols must retain object identity, not consult a
        # global name-only visibility/address table. Decode both input orders.
        local_objects = []
        markers = [(0x129be874e361fa05, 0x6cae48d7902b135f),
                   (0x54be71a36289fc0d, 0x9b34df07e26518ac)]
        for i, (site_mark, data_mark) in enumerate(markers):
            local_src, local_obj = w / f"local{i}.s", w / f"local{i}.o"
            local_src.write_text(f".text\n.quad {site_mark}\nadr x0,target\nret\n"
                                 f".data\n.local target\ntarget: .quad {data_mark}\n")
            run(["-c", local_src, "-o", local_obj])
            local_objects.append(local_obj)
        for order in [local_objects, local_objects[::-1]]:
            run(["-nostdlib", "-shared", *order, "-o", exe])
            for site_mark, data_mark in markers:
                data, site, p, _ = mapped_marker(exe, site_mark, 8)
                _, _, target, _ = mapped_marker(exe, data_mark)
                assert p + displacement(struct.unpack_from("<I", data, site)[0]) == target
                checks += 1
        # AArch64 does not yet serialize/enforce merged ELF visibility across
        # dynsym/GOT/PLT. Bound shared ADR to true locals, in either input order.
        definition, definition_obj = w / "definition.s", w / "definition.o"
        src.write_text(".text\n.global _start\n_start: adr x0,target\nret\n")
        run(["-c", src, "-o", obj])
        for visibility in ["hidden", "protected"]:
            definition.write_text(f".data\n.global target\n.{visibility} target\ntarget: .quad 1\n")
            run(["-c", definition, "-o", definition_obj])
            for inputs in [[obj, definition_obj], [definition_obj, obj]]:
                p = run(["-shared", "-nostdlib", *inputs, "-o", w / "hidden.so"], False)
                assert "local allocated section-relative" in p.stderr, p.stderr
                checks += 1
        # Same-section ADRP must account for the final merged section's page.
        definition.write_text(".text\n.space 4080\n")
        run(["-c", definition, "-o", definition_obj])
        src.write_text(".text\ntarget: .quad 0x52fd03498abc7e91\n"
                       ".quad 0x8c67d1284e9ab053\n.global _start\n_start: adrp x0,target\nret\n")
        run(["-c", src, "-o", obj])
        exe = w / "pages.elf"
        run(["-nostdlib", definition_obj, obj, "-o", exe])
        binary, site, p, _ = mapped_marker(exe, 0x8c67d1284e9ab053, 8)
        _, _, target, _ = mapped_marker(exe, 0x52fd03498abc7e91)
        if p >> 12 == target >> 12:
            # Native text may follow headers rather than start at page offset
            # zero. Adjust the prefix using the actual first link placement.
            padding = 4080 + ((4080 - (target & 4095)) % 4096)
            definition.write_text(f".text\n.space {padding}\n")
            run(["-c", definition, "-o", definition_obj])
            run(["-nostdlib", definition_obj, obj, "-o", exe])
            binary, site, p, _ = mapped_marker(exe, 0x8c67d1284e9ab053, 8)
            _, _, target, _ = mapped_marker(exe, 0x52fd03498abc7e91)
        assert p >> 12 != target >> 12, "fixture must cross a final page"
        word = struct.unpack_from("<I", binary, site)[0]
        assert displacement(word) == (target >> 12) - (p >> 12)
        checks += 1

        # Shared ADR cannot implement preemption or an absolute fixed address.
        for binding in [".global target", ".weak target"]:
            src.write_text(f".text\n.global _start\n_start: adr x0,target\nret\n.data\n{binding}\ntarget: .quad 0\n")
            run(["-c", src, "-o", obj])
            run(["-nostdlib", "-shared", obj, "-o", exe], False)
            checks += 1
    print(f"PASS: AArch64 address contract ({checks} checks)")


if __name__ == "__main__":
    main()
