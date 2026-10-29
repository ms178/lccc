#!/usr/bin/env python3
"""Compare i386 DWARF GOTOFF fixups against the GNU linker and ELF symbols.

This exercises S + A - GOT for local and global symbols, positive and
negative addends, and static executable, PIE, and shared-object links.  Linker
layouts differ, so the check independently calculates the expected field from
each output's own symbols. A debug-only GOTOFF with no GOTPC is deliberately
still rejected by LCCC; GNU is verified to accept it.
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


def fields(path, cwd):
    run(["objcopy", "--dump-section", ".debug_loclists=section.bin", path], cwd)
    with open(os.path.join(cwd, "section.bin"), "rb") as f:
        data = f.read()
    if len(data) != 12:
        raise AssertionError(f"{path}: expected exactly 12 debug bytes, got {len(data)}")
    return struct.unpack("<III", data)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--lccc-ld", required=True)
    args = parser.parse_args()
    linker = os.path.abspath(args.lccc_ld)
    template = '''\n.globl _start\n.text\n_start:\n  xorl %eax,%eax\n  {got}\n  ret\n.data\n.space 17\n.globl foo\nfoo: .long 42\nlocal_data: .long 23\n.section .debug_loclists,"",@progbits\n.long foo@GOTOFF+7\n.long local_data@GOTOFF-4\n.long foo@GOTOFF-8\n.section .note.GNU-stack,"",@progbits\n'''
    with tempfile.TemporaryDirectory(prefix="i686-debug-gotoff-") as td:
        for got in (True, False):
            source = template.format(got="movl $_GLOBAL_OFFSET_TABLE_,%ebx" if got else "")
            with open(os.path.join(td, "fixture.s"), "w", encoding="utf-8") as f:
                f.write(source)
            run(["as", "--32", "fixture.s", "-o", "fixture.o"], td)
            for mode in ("-static", "-pie", "-shared"):
                for name, ld in (("GNU", "ld"), ("LCCC", linker)):
                    cmd = [ld, "-m", "elf_i386", mode, "-o", "image", "fixture.o"]
                    if not got and name == "LCCC":
                        p = subprocess.run(cmd, cwd=td, stdout=subprocess.PIPE,
                                           stderr=subprocess.PIPE, text=True)
                        if p.returncode == 0 or "relocation type 9" not in p.stderr:
                            raise AssertionError(f"{mode} debug-only: expected explicit rejection: {p.stderr}")
                        continue
                    run(cmd, td)
                    actual = fields("image", td)
                    if got:
                        syms = symbols("image", td)
                        for sym in ("foo", "local_data", "_GLOBAL_OFFSET_TABLE_"):
                            if sym not in syms:
                                raise AssertionError(f"{name} {mode}: missing symbol {sym}")
                        base = syms["_GLOBAL_OFFSET_TABLE_"]
                        want = tuple(v & 0xffffffff for v in (
                            syms["foo"] + 7 - base,
                            syms["local_data"] - 4 - base,
                            syms["foo"] - 8 - base))
                    else:
                        # GNU may synthesize an unexported anchor even with
                        # no GOT section; do not guess that anchor here.
                        print(f"PASS {name} {mode} debug-only link {actual}")
                        continue
                    if actual != want:
                        raise AssertionError(f"{name} {mode} GOTPC={got}: {actual!r} != {want!r}")
                    print(f"PASS {name} {mode} GOTPC={got} {actual}")
    print("PASS i686 debug GOTOFF oracle comparison")


if __name__ == "__main__":
    main()
