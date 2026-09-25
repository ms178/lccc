#!/usr/bin/env bash
# ISA-parity audit: i686 gains the legacy-encoding integer ISA forms
# (tzcnt F3 0F BC — BSF-decoding on pre-ABM hardware; popcnt F3 0F B8)
# under EXPLICIT flags only, with the -mno-* veto and the macro contract
# (__LZCNT__/__POPCNT__ mirror exactly what codegen may emit).
# movbe/bmi1 stay x86-64-only: the i686 backend has no consumers for
# them yet (granting a capability without a serving emitter reopens the
# middle-end deferral cliff PR #607 closed).
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-i686isa.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/t.c" <<'C'
typedef unsigned int u32;
u32 ctz(u32 x) { return (u32)__builtin_ctz(x); }
u32 pop(u32 x) { return (u32)__builtin_popcount(x); }
int main(void) { return (int)(ctz(8u) + pop(7u)); }
C

# 1. Explicit flags: the forms are used, and the program still runs.
"$CCC" -O2 -m32 -mlzcnt -mpopcnt -S "$tmp/t.c" -o "$tmp/on.s"
grep -q 'tzcntl' "$tmp/on.s"
grep -q 'popcntl' "$tmp/on.s"
"$CCC" -O2 -m32 -mlzcnt -mpopcnt "$tmp/t.c" -o "$tmp/on"
set +e
"$tmp/on"
st=$?
set -e
# ctz(8)+pop(7) == 6: the ABM/SSE4.2 forms compute correctly at runtime.
[ "$st" -eq 6 ]

# 2. Baseline -m32: broad-compatibility contract — no ABM/SSE4.2 forms.
"$CCC" -O2 -m32 -S "$tmp/t.c" -o "$tmp/base.s"
! grep -qE 'tzcnt|popcnt' "$tmp/base.s"

# 3. The -mno-* veto wins over the explicit grant.
"$CCC" -O2 -m32 -mlzcnt -mno-lzcnt -S "$tmp/t.c" -o "$tmp/veto.s"
! grep -q 'tzcnt' "$tmp/veto.s"
"$CCC" -O2 -m32 -mpopcnt -mno-popcnt -S "$tmp/t.c" -o "$tmp/veto2.s"
! grep -q 'popcnt' "$tmp/veto2.s"

# 4. Macro contract mirrors codegen exactly (GCC defines the same set).
"$CCC" -O2 -m32 -mlzcnt -mpopcnt -E "$tmp/t.c" 2>/dev/null | head -1 >/dev/null
"$CCC" -O2 -m32 -mlzcnt -mpopcnt -S -o "$tmp/m1.s" /dev/null 2>/dev/null || true
printf '#ifdef __LZCNT__\nint lz_m;\n#endif\n#ifdef __POPCNT__\nint pc_m;\n#endif\nint main(void){return 0;}\n' > "$tmp/m.c"
"$CCC" -O2 -m32 -mlzcnt -mpopcnt -S "$tmp/m.c" -o "$tmp/m1.s"
grep -q 'lz_m' "$tmp/m1.s" && grep -q 'pc_m' "$tmp/m1.s"
"$CCC" -O2 -m32 -S "$tmp/m.c" -o "$tmp/m2.s"
! grep -q 'lz_m' "$tmp/m2.s"
! grep -q 'pc_m' "$tmp/m2.s"

# 5. x86-64 semantics unchanged: default grant still on, -mno still wins.
"$CCC" -O2 -S "$tmp/t.c" -o "$tmp/x64.s"
grep -qE 'tzcnt|popcnt' "$tmp/x64.s"
"$CCC" -O2 -mno-lzcnt -S "$tmp/t.c" -o "$tmp/x64v.s"
! grep -q 'tzcnt' "$tmp/x64v.s"

echo "i686 integer-ISA parity gate: PASS"
