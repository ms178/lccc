# A14-31: distinct non-UTF-8 ELF symbol names

Original audit fixture (not copied from a third-party project). Reproduce from
this directory using scratch outputs, not tracked binaries:

```sh
work=$(mktemp -d)
as input.s -o "$work/input.o"
python3 - "$work/input.o" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
b = p.read_bytes()
assert b.count(b'symA\0') == 1 and b.count(b'symB\0') == 1
p.write_bytes(b.replace(b'symA\0', b'sym\xff\0').replace(b'symB\0', b'sym\xfe\0'))
PY
ld "$work/input.o" -o "$work/bfd"
"$work/bfd"  # status 0
ld.lld "$work/input.o" -o "$work/lld"
"$work/lld"  # status 0
# Use the absolute path to the already-built fastbuild binary:
LCCC_LD=/absolute/path/to/lccc/target/fastbuild/lccc-ld
"$LCCC_LD" "$work/input.o" -o "$work/lccc"
# Current baseline: status 1, multiple definition of 'sym�'.
rm -rf "$work"
```

Executed with distro bfd 2.44 and lld 19.1.7 (see parent environment.json).
This is byte-identity/acceptance evidence, not a pinned-linker performance result.
