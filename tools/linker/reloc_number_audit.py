#!/usr/bin/env python3
"""Audit every relocation-type constant defined in the lccc sources against the
authoritative numbers in the system ELF header.

A wrong relocation number is one of the few bugs a linker cannot recover from at
runtime: the constant is how an arm in a match is selected, so a type that is
off by one is handled by the wrong arm and writes the wrong value into the image
-- or is not handled at all. Nothing downstream can notice, because the number
never appears in the output. There is also no way to spot it by reading code,
since the wrong number looks exactly like the right one.

So the check is mechanical: parse `#define R_*` out of <elf.h>, parse every
`const R_*: u32 = N;` out of the sources, and compare. Duplicates within one file
and disagreements between files are both reported, because a backend that defines
its own copy of a constant is a backend that can drift from the ABI and from the
other backends.
"""
import collections
import pathlib
import re
import sys

ELF_H = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "/usr/include/elf.h")
ROOT = pathlib.Path(sys.argv[2] if len(sys.argv) > 2 else "/home/user/lccc")

truth = {}
for line in ELF_H.read_text().splitlines():
    m = re.match(r"\s*#define\s+(R_(?:X86_64|386|AARCH64|RISCV)_[A-Z0-9_]+)\s+(\d+)", line)
    if m:
        truth[m.group(1)] = int(m.group(2))
print(f"{ELF_H}: {len(truth)} authoritative relocation numbers")

found = collections.defaultdict(list)  # name -> [(file, line, value)]
pat = re.compile(r"\b(?:pub\s+)?const\s+(R_(?:X86_64|386|AARCH64|RISCV)_[A-Z0-9_]+)\s*:\s*u\d+\s*=\s*(\d+)")
srcs = sorted(p for p in ROOT.rglob("*.rs") if "target" not in p.parts)
for p in srcs:
    try:
        text = p.read_text()
    except (UnicodeDecodeError, OSError):
        continue
    for i, line in enumerate(text.splitlines(), 1):
        m = pat.search(line)
        if m:
            found[m.group(1)].append((str(p.relative_to(ROOT)), i, int(m.group(2))))

bad = []
unknown = []
dupes = []
for name, sites in sorted(found.items()):
    if name not in truth:
        unknown.append((name, sites))
        continue
    for f, ln, val in sites:
        if val != truth[name]:
            bad.append((name, val, truth[name], f, ln))
    vals = {v for _f, _l, v in sites}
    if len(vals) > 1:
        dupes.append((name, sites))

print(f"sources scanned: {len(srcs)}; distinct constants defined: {len(found)}; "
      f"definitions: {sum(len(v) for v in found.values())}\n")

print(f"=== WRONG NUMBERS: {len(bad)} ===")
for name, got, want, f, ln in bad:
    print(f"  {f}:{ln}  {name} = {got}   elf.h says {want}"
          + (f"   ({want} is really {next((k for k,v in truth.items() if v==want and k.split('_')[1]==name.split('_')[1]), '?')})" if True else ""))

print(f"\n=== NOT IN elf.h (possible typos or vendor extensions): {len(unknown)} ===")
for name, sites in unknown:
    print(f"  {name}: " + ", ".join(f"{f}:{ln}={v}" for f, ln, v in sites))

print(f"\n=== DEFINED INCONSISTENTLY BETWEEN FILES: {len(dupes)} ===")
for name, sites in dupes:
    print(f"  {name}: " + ", ".join(f"{f}:{ln}={v}" for f, ln, v in sites))

sys.exit(1 if bad or dupes else 0)
