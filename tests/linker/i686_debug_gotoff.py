#!/usr/bin/env python3
"""Compare i386 DWARF GOTOFF fixups against the GNU linker and ELF symbols.

Check S + A - GOT with positive/negative addends and static, PIE and shared
links, both with and without code relocations requiring a GOT or PLT. Linker
layouts differ, so each result is checked against its own symbol addresses.
Also check undefined weak references and symbols resolved from another object.
"""
import argparse
import os
import re
import struct
import subprocess
import tempfile


def run(cmd, cwd, expected=0):
    p = subprocess.run(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if p.returncode != expected:
        raise AssertionError(f"{cmd}: rc={p.returncode}, wanted {expected}: {p.stderr}")
    return p


def symbols(path, cwd):
    out = run(["readelf", "-sW", path], cwd).stdout
    result = {}
    for line in out.splitlines():
        fields = line.split()
        if len(fields) >= 8 and re.fullmatch(r"[0-9]+:", fields[0]):
            if fields[6] != "UND":
                result[fields[7]] = int(fields[1], 16)
    return result


def fields(path, cwd, count=3):
    run(["objcopy", "--dump-section", ".debug_loclists=section.bin", path], cwd)
    with open(os.path.join(cwd, "section.bin"), "rb") as f:
        data = f.read()
    if len(data) != 4 * count:
        raise AssertionError(f"{path}: expected {4 * count} debug bytes, got {len(data)}")
    return struct.unpack(f"<{count}I", data)


def check_values(name, mode, kind, cwd, weak=False):
    actual = fields("image", cwd, 4 if weak else 3)
    syms = symbols("image", cwd)
    for sym in ("foo", "local_data"):
        if sym not in syms:
            raise AssertionError(f"{name} {mode} {kind}: missing symbol {sym}")
    if "_GLOBAL_OFFSET_TABLE_" in syms:
        base = syms["_GLOBAL_OFFSET_TABLE_"]
    elif name == "GNU" and kind != "gotpc":
        # GNU can use an implicit, unexported anchor when no GOTPC refers
        # to it. Derive it from one field, then check the remaining fields.
        base = (syms["foo"] + 7 - actual[0]) & 0xffffffff
    else:
        raise AssertionError(f"{name} {mode} {kind}: missing GOT anchor")
    expected = [syms["foo"] + 7 - base, syms["local_data"] - 4 - base,
                syms["foo"] - 8 - base]
    if weak:
        expected.append(4 - base)  # Undefined weak symbol resolves to zero.
    want = tuple(v & 0xffffffff for v in expected)
    if actual != want:
        raise AssertionError(f"{name} {mode} {kind}: {actual!r} != {want!r}")
    print(f"PASS {name} {mode} {kind}{' weak' if weak else ''} {actual}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--lccc-ld", required=True)
    args = parser.parse_args()
    linker = os.path.abspath(args.lccc_ld)
    template = '''\n.globl _start, external_fn\n.text\n_start:\n  xorl %eax,%eax\n  {code}\n  ret\nexternal_fn: ret\n{data}\n{weak_decl}\n.section .debug_loclists,"",@progbits\n.long foo@GOTOFF+7\n.long local_data@GOTOFF-4\n.long foo@GOTOFF-8\n{weak_reloc}\n.section .note.GNU-stack,"",@progbits\n'''
    data = '.data\n.space 17\n.globl foo\nfoo: .long 42\nlocal_data: .long 23'
    cases = (("no-got", ""),
             ("gotpc", "movl $_GLOBAL_OFFSET_TABLE_,%ebx"),
             ("got-entry", "movl foo@GOT(%ebx),%eax"),
             ("plt-entry", "call external_fn@PLT"))
    with tempfile.TemporaryDirectory(prefix="i686-debug-gotoff-") as td:
        for kind, code in cases:
            for weak in (False, True):
                source = template.format(code=code, data=data,
                    weak_decl=".weak weak_missing" if weak else "",
                    weak_reloc=".long weak_missing@GOTOFF+4" if weak else "")
                with open(os.path.join(td, "fixture.s"), "w", encoding="utf-8") as f:
                    f.write(source)
                run(["as", "--32", "fixture.s", "-o", "fixture.o"], td)
                for mode in ("-static", "-pie", "-shared"):
                    for name, ld in (("GNU", "ld"), ("LCCC", linker)):
                        cmd = [ld, "-m", "elf_i386", mode, "-o", "image", "fixture.o"]
                        if weak and mode == "-shared":
                            p = subprocess.run(cmd, cwd=td, stdout=subprocess.PIPE,
                                               stderr=subprocess.PIPE, text=True)
                            if p.returncode == 0 or "R_386_GOTOFF" not in p.stderr:
                                raise AssertionError(f"{name} {mode} {kind}: expected rejection: {p.stderr}")
                            print(f"PASS {name} {mode} {kind} undefined weak rejected")
                        else:
                            run(cmd, td)
                            check_values(name, mode, kind, td, weak)

        # The reference is undefined in the debug object's symbol table, but
        # resolves statically in another object; shared links must accept it.
        source = template.format(code="", data="", weak_decl="", weak_reloc="")
        with open(os.path.join(td, "fixture.s"), "w", encoding="utf-8") as f:
            f.write(source)
        with open(os.path.join(td, "defs.s"), "w", encoding="utf-8") as f:
            f.write('.data\n.space 17\n.globl foo,local_data\nfoo: .long 42\n'
                    'local_data: .long 23\n.section .note.GNU-stack,"",@progbits\n')
        for stem in ("fixture", "defs"):
            run(["as", "--32", f"{stem}.s", "-o", f"{stem}.o"], td)
        for mode in ("-static", "-pie", "-shared"):
            for name, ld in (("GNU", "ld"), ("LCCC", linker)):
                run([ld, "-m", "elf_i386", mode, "-o", "image", "fixture.o", "defs.o"], td)
                check_values(name, mode, "cross-object", td)
    print("PASS i686 debug GOTOFF oracle comparison")


if __name__ == "__main__":
    main()
