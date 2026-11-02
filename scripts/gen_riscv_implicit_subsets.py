#!/usr/bin/env python3
"""Regenerate the RISC-V implicit-subset table from GNU binutils.

Usage: gen_riscv_implicit_subsets.py <binutils-src>/bfd/elfxx-riscv.c

Prints the Rust `IMPLICIT` table used by
src/backend/riscv/linker/attributes.rs.  bfd's link-time arch merge
needs only which subsets are *present* after the closure (the conflict
checks); implied subsets get no version there and are never printed.
"""
import re
import sys

src = open(sys.argv[1], encoding="utf-8").read()
m = re.search(r"riscv_implicit_subsets\[\]\s*=\s*\{(.*?)\n\};", src, re.S)
if not m:
    sys.exit("riscv_implicit_subsets[] not found")
checks = {
    "check_implicit_always": "Always",
    "check_implicit_for_i": "IBelow2p1",
    "check_implicit_for_zcf": "Rv32WithF",
    "check_implicit_for_zcd": "WithD",
}
rows = re.findall(r'\{\s*"([a-z0-9]+)",\s*"([^"]*)",\s*(\w+)\s*\}', m.group(1))
print("const IMPLICIT: &[(&str, &[&str], Check)] = &[")
for ext, imps, chk in rows:
    names = []
    for item in imps.split(","):
        if not item.startswith("+"):
            sys.exit(f"unsupported implicit entry {ext}: {item}")
        names.append(item[1:])
    if chk not in checks:
        sys.exit(f"unknown check function {chk}")
    body = ", ".join(f'"{n}"' for n in names)
    print(f'    ("{ext}", &[{body}], Check::{checks[chk]}),')
print("];")
print(f"// {len(rows)} rows", file=sys.stderr)
