#!/usr/bin/env bash
# Session-04 (Cachymod 6.18.47 boot): a C `section` attribute on a
# zero-initialized global must emit PROGBITS zero bytes, NOT @nobits.
#
# Root cause: emit_globals() marked EVERY zero-initialized writable custom
# section @nobits.  GCC (verified 14.2 objects + 16.2 Godbolt asm) always
# emits PROGBITS for a section attribute; when the SAME TU also places an
# initialized member in that section (kernel .init.data, .data..percpu,
# .data..read_mostly all do), the writer hit "changed section type for
# .init.data" and init/main.c failed to compile.  GAS 2.44/2.47 confirm the
# type-conflict diagnostic is correct assembler behaviour — the generator was
# wrong, not the writer.
#
# Only .bss-named sections stay NOBITS (GNU as assigns SHT_NOBITS by name).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
OBJDUMP=${OBJDUMP:-objdump}
READLF=${READLF:-readelf}
tmp=${TMPDIR:-/tmp}/lccc-sect.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

# Mixed zero-initialized + initialized members in one custom section
# (exact shape of init/main.c's .init.data).
cat >"$tmp/t.c" <<'C'
__attribute__((section(".init.data"))) static int zerovar;
__attribute__((section(".init.data"))) static int onevar = 5;
__attribute__((section(".init.data"))) static int cv = 7;
int use(void) { return zerovar + onevar + cv; }
C
"$CCC" -O2 -c "$tmp/t.c" -o "$tmp/t.o"

# 1. The object has exactly ONE .init.data and it is PROGBITS (GCC parity:
#    readelf on GCC's output shows `.init.data PROGBITS` with .zero bytes).
types=$("$READLF" -S -W "$tmp/t.o" | awk '$3==".init.data"{print $4}')
[ "$types" = "PROGBITS" ] || { echo "FAIL: .init.data type=$types (want PROGBITS)"; exit 1; }
count=$("$READLF" -S -W "$tmp/t.o" | awk '$3==".init.data"' | wc -l)
[ "$count" -eq 1 ] || { echo "FAIL: $count .init.data sections"; exit 1; }

# 2. All three members live in that section and the content is real bytes
#    (zero fill present, not dropped): onevar=5, cv=7 must be findable.
"$OBJDUMP" -s -j .init.data "$tmp/t.o" > "$tmp/dump.txt"
grep -q "05000000" "$tmp/dump.txt" || { echo "FAIL: onevar content missing"; exit 1; }
grep -q "07000000" "$tmp/dump.txt" || { echo "FAIL: cv content missing"; exit 1; }

# 3. Runtime sanity: zero-init really is zero in a PROGBITS custom section.
cat >"$tmp/m.c" <<'C'
#include <stdio.h>
__attribute__((section(".mysec"))) static int z;
__attribute__((section(".mysec"))) static int v = 41;
int main(void) { int r = z + v; printf("%d\n", r); return r != 41; }
C
"$CCC" -O2 "$tmp/m.c" -o "$tmp/m"
out=$("$tmp/m")
[ "$out" = "41" ] || { echo "FAIL: runtime got $out"; exit 1; }

# 4. .bss-named custom sections remain NOBITS (kernel .bss..page_aligned).
cat >"$tmp/b.c" <<'C'
__attribute__((section(".bss..myaligned"), aligned(4096))) static char page[4096];
char *get(void) { return page; }
C
"$CCC" -O2 -c "$tmp/b.c" -o "$tmp/b.o"
btype=$("$READLF" -S -W "$tmp/b.o" | awk '$3==".bss..myaligned"{print $4}')
[ "$btype" = "NOBITS" ] || { echo "FAIL: .bss..myaligned type=$btype (want NOBITS)"; exit 1; }

echo "PASS: custom-section zero-init is PROGBITS (GCC parity); .bss* stays NOBITS"
