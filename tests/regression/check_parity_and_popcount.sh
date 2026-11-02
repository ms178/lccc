#!/usr/bin/env bash
# Parity is the population count modulo two -- a different question from the
# count, and every architecture answers it shorter:
#
#   x86 / i686   fold the operand to a byte with XOR (`x ^ x>>16` then
#                `xor %ah, %al`) and read PF, the parity flag of that byte:
#                `setnp` inverts it to the builtin's 1-for-odd.
#   AArch64      with SIMD, `cnt`/`uaddlv` and `and #1` (five instructions);
#                without, `eor w, w, w, lsr` folded all the way down to bit 0
#                (one instruction per fold -- the ISA has a shifted operand)
#                and masked.  The nibble table (0x6996) costs one more there,
#                because it needs `and #0xf`, a `mov` of the constant and a
#                variable `lsr`: the table is the RISC-V form, where a fold
#                costs two instructions instead of one.
#   RISC-V       the same nibble table (`li t2, 0x6996` + `srl`), because the
#                ISA has no parity flag and no scalar population count.
#
# `__builtin_parity` used to lower as `Popcount(x) & 1`, so every backend paid
# for the full count and then masked it.  Measured on x86-64 at -O2: 17
# instructions against GCC 16.2's 6 and Clang 23.1.4's 6.  The same round made
# RISC-V's own `Popcount` branch-free (Kernighan's loop was five instructions
# per set bit behind a data-dependent branch), which is why the loop idiom may
# now fold to `Popcount` on RISC-V (`resolved_fast_popcount`).
#
# The narrow shapes are a separate question, and the round after the parity
# lowering answered them: a count's value only depends on the operand's
# significant bits, so
#
#   * `popcountll(x) & 1` -- the low bit of a count is the parity of the
#     operand at the *operand's* width, whatever width the C `int` mask was
#     narrowed to, so the 64-bit scan plus `andl $1` becomes the 64-bit parity
#     (x86-64: 22 instructions -> 11, the same body `__builtin_parityll` emits);
#   * `__builtin_parity(uchar)` promotes its argument, so the count runs over
#     `zext(x)` at 32 bits; running it at the byte's own width is two XORs and
#     a flag (8 -> 5), and `parity(u16)` likewise (8 -> 5);
#   * a byte's population count needs neither a 24-bit scan nor a scratch
#     register: `imul $0x08040201` / `shr $3` / `and $0x11111111` /
#     `imul $0x11111111` / `shr $28` is exact for all 256 inputs and takes the
#     count from 17 to 8 (i686: 19 -> 11).
#
# This gate pins eight things:
#
#   1. the x86-64 baseline shape is the flag sequence -- `setnp` present and
#      the SWAR count's `imul` absent from every parity function,
#   2. `-mpopcnt` selects `popcnt` for both parity and count,
#   3. `CCC_SIMPLIFY_SKIP=parity` puts `__builtin_popcount(x) & 1` back on the
#      count (the `imul` returns), which is what proves the fold above belongs
#      to the parity rule and not to some other one -- while
#      `__builtin_popcountll(x) & 1`, whose mask sits at a different width, is
#      still folded: that one belongs to the bit-idiom count-width rule,
#   4. the other three backends emit their own shape (`setnp` on i686, the
#      0x6996 table on AArch64-with-`-mgeneral-regs-only` and on RISC-V) and
#      their per-function counts stay at the pinned numbers,
#   5. RISC-V's `Popcount` is straight-line: no branch, no jump, no call, and
#      the SWAR masks are present,
#   6. a byte's count is the five-instruction multiplier form on both x86
#      backends: two `imul`s, no `push`/`pop` staging, no 0x55555555 mask,
#   7. the narrow-count rule is a target cost decision, so the back ends that
#      widen a narrow result for free (x86-64, i686) narrow and the others keep
#      the promoted form: AArch64's and RISC-V's `par8`/`pc8`/`pc16` are
#      unchanged from the pre-round numbers,
#   8. the values agree with an independent model at -O0/-O1/-O2/-O3/-Os and
#      with GCC, bit for bit, over every 8- and 16-bit input, single bits at
#      every position of both widths, and 4096 arithmetic operands.
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
bindir=$(CDPATH= cd -- "$(dirname -- "$CCC")" && pwd)
tmp=${TMPDIR:-/tmp}/lccc-parity.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat > "$tmp/shapes.c" <<'C'
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned u32;
typedef unsigned long u64;
int par8(u8 x)        { return __builtin_parity(x); }
int par16(u16 x)      { return __builtin_parity(x); }
int par32(u32 x)      { return __builtin_parity(x); }
int par64(u64 x)      { return __builtin_parityl(x); }
int par64ll(u64 x)    { return __builtin_parityll(x); }
int pc8(u8 x)         { return __builtin_popcount(x); }
int pc16(u16 x)       { return __builtin_popcount(x); }
int pc32(u32 x)       { return __builtin_popcount(x); }
int pc64(u64 x)       { return __builtin_popcountl(x); }
/* The idiom the simplify rule exists for: a parity flag spelled as the low
 * bit of a count. */
int par_of_count(u32 x){ return __builtin_popcount(x) & 1; }
int par_of_count64(u64 x){ return (int)(__builtin_popcountll(x) & 1); }
C

# Instruction lines of one function in `-S` output: no directives, labels,
# comments or blank lines.
fn_body() {
    awk -v F="^$1:" '$0 ~ F {p=1; next} p && /^[[:space:]]*\.size/ {exit} p' "$2" |
        grep -vE '^[[:space:]]*(\.|#|$)' | grep -vE '^[A-Za-z_.$][A-Za-z0-9_.$]*:' || true
}
count_of() { fn_body "$1" "$2" | wc -l | tr -d ' '; }

# Also a check annotation: the Actions log download endpoint is not always
# reachable from API clients, but the Checks API is, so a red lane here stays
# diagnosable (the message is flattened to one line -- a workflow command
# cannot carry newlines).
fail() {
    echo "FAIL: $*" >&2
    printf '::error title=check_parity_and_popcount::%s\n' "${*//$'\n'/ | }"
    exit 1
}

# ---- 1/2/3. x86-64 --------------------------------------------------------
x86=$bindir/lccc
[ -x "$x86" ] || x86=$CCC
"$x86" -O2 -S "$tmp/shapes.c" -o "$tmp/x86.s"
# `par_of_count64` is the cross-width shape: the frontend narrows the `& 1` to
# 32 bits (a legitimate narrowing -- AND with a mask below 2^32 reads only the
# low half), so the count is 64-bit and the mask 32-bit.  The simplifier's
# parity rule insists on equal *sizes*, so this one is folded by the bit-idiom
# count-width rule instead, and it lands on exactly the body
# `__builtin_parityll` emits.
for f in par8 par16 par32 par64 par64ll par_of_count par_of_count64; do
    body=$(fn_body "$f" "$tmp/x86.s")
    grep -q 'setnp' <<<"$body" || fail "$f: no setnp in the parity lowering"
    if grep -qE '\bimul' <<<"$body"; then
        fail "$f still carries the SWAR population count"
    fi
    grep -qE '\bj[a-z]+\b|\bcall' <<<"$body" && fail "$f is not straight-line"
done
# Popcount itself keeps the SWAR (it must produce the whole sum).
body=$(fn_body pc32 "$tmp/x86.s")
grep -qE '\bimul' <<<"$body" || fail "pc32 lost the SWAR count"
grep -qE '\bj[a-z]+\b|\bcall' <<<"$body" && fail "pc32 is not branch-free"

# A byte's count: the five-instruction multiplier form.  Two multipliers (the
# spread and the nibble sum), no 32-bit SWAR mask, and no scratch staging --
# the count of eight bits needs no 24-bit scan and no second register.
body=$(fn_body pc8 "$tmp/x86.s")
[ "$(grep -cE '\bimull\b' <<<"$body")" = 2 ] || fail "pc8 is not the multiplier form"
grep -q '0x55555555' <<<"$body" && fail "pc8 still runs the 32-bit SWAR"
grep -qE '\bpushq?\b|\bpopq?\b' <<<"$body" && fail "pc8 stages a scratch register"
grep -qE '\bj[a-z]+\b|\bcall' <<<"$body" && fail "pc8 is not straight-line"

"$x86" -O2 -mpopcnt -S "$tmp/shapes.c" -o "$tmp/x86.popcnt.s"
for f in par32 pc32; do
    grep -q 'popcntl' <<<"$(fn_body "$f" "$tmp/x86.popcnt.s")" ||
        fail "$f did not select popcnt under -mpopcnt"
done

CCC_SIMPLIFY_SKIP=parity "$x86" -O2 -S "$tmp/shapes.c" -o "$tmp/x86.nofold.s"
grep -qE '\bimul' <<<"$(fn_body par_of_count "$tmp/x86.nofold.s")" ||
    fail "CCC_SIMPLIFY_SKIP=parity did not restore the count in par_of_count"
grep -q 'setnp' <<<"$(fn_body par_of_count "$tmp/x86.nofold.s")" &&
    fail "CCC_SIMPLIFY_SKIP=parity left the parity shape in place"
# ... but the cross-width shape is not the simplifier's: it stays folded.
grep -q 'setnp' <<<"$(fn_body par_of_count64 "$tmp/x86.nofold.s")" ||
    fail "CCC_SIMPLIFY_SKIP=parity lost the count-width fold (par_of_count64)"
grep -qE '\bimull\b' <<<"$(fn_body par_of_count64 "$tmp/x86.nofold.s")" &&
    fail "par_of_count64 fell back to the 64-bit count under the skip"
# The builtin itself is not the knob's business: it must stay folded.
grep -q 'setnp' <<<"$(fn_body par32 "$tmp/x86.nofold.s")" ||
    fail "CCC_SIMPLIFY_SKIP=parity changed __builtin_parity itself"

# Pinned counts, `-S` lines including the return and the frame the emitter
# builds around each shape (the algorithm itself is 7 instructions for a 32-bit
# parity, 9 for 64, and these numbers are what the whole function costs).
pin() {  # function, file, expected
    local got; got=$(count_of "$1" "$2")
    [ "$got" = "$3" ] || fail "$1: $got instructions, pinned $3"
}
pin par32 "$tmp/x86.s" 8
pin par64 "$tmp/x86.s" 11
# A byte's parity is one `xorb %ah, %al` after the load: 8 -> 5.
pin par8  "$tmp/x86.s" 5
pin par16 "$tmp/x86.s" 5
pin par_of_count "$tmp/x86.s" 8
# The cross-width shape lands on the 64-bit parity body itself (par64), not on
# the 64-bit count plus a mask, which used to cost 22.
pin par_of_count64 "$tmp/x86.s" 11
pin pc32  "$tmp/x86.s" 17
# A byte's count is the multiplier form: 17 -> 8, and the 16-bit one keeps the
# 32-bit scan (narrowing it would only add the extension).
pin pc8   "$tmp/x86.s" 8
pin pc16  "$tmp/x86.s" 17

# ---- 4. i686, AArch64, RISC-V --------------------------------------------
i686=$bindir/lccc-i686; arm=$bindir/lccc-arm; rv=$bindir/lccc-riscv
for t in "$i686" "$arm" "$rv"; do
    [ -x "$t" ] || fail "missing backend driver $t"
done
"$i686" -O2 -S "$tmp/shapes.c" -o "$tmp/i686.s"
for f in par32 par_of_count; do
    grep -q 'setnp' <<<"$(fn_body "$f" "$tmp/i686.s")" || fail "$f: no setnp on i686"
done

"$arm" -O2 -mgeneral-regs-only -S "$tmp/shapes.c" -o "$tmp/arm.gpr.s"
for f in par32 par64; do
    body=$(fn_body "$f" "$tmp/arm.gpr.s")
    # The eor chain to bit 0: one fold per width halving, then the mask.
    grep -qE 'eor\s+w1, w1, w1, lsr #8' <<<"$body" || fail "$f: no byte fold on AArch64"
    grep -qE 'eor\s+w1, w1, w1, lsr #1' <<<"$body" || fail "$f: no fold to bit 0 on AArch64"
    grep -qE 'and\s+w0, w1, #1' <<<"$body" || fail "$f: no mask of bit 0 on AArch64"
    # ... and it must not pay for the table, which is the RISC-V form.
    grep -q '0x6996' <<<"$body" && fail "$f: table lookup on AArch64 (the fold is shorter)"
    grep -qE '\bb\.?[a-z]*\s' <<<"$body" && fail "$f is not straight-line on AArch64"
done
# Six instructions for 32 bits, seven for 64 -- one fewer than the table on
# both, and one fewer than Clang's 64-bit form.
armbody32=$(fn_body par32 "$tmp/arm.gpr.s")
[ "$(grep -cE '^\s+(eor|and)\s' <<<"$armbody32")" = 6 ] ||
    fail "par32: AArch64 fold is not 6 instructions"
armbody64=$(fn_body par64 "$tmp/arm.gpr.s")
[ "$(grep -cE '^\s+(eor|and)\s' <<<"$armbody64")" = 7 ] ||
    fail "par64: AArch64 fold is not 7 instructions"
"$arm" -O2 -S "$tmp/shapes.c" -o "$tmp/arm.s"
grep -q '\bcnt\b' <<<"$(fn_body par32 "$tmp/arm.s")" ||
    fail "AArch64 did not use the SIMD count for parity"

"$rv" -O2 -S "$tmp/shapes.c" -o "$tmp/rv.s"
for f in par32 par64 par64ll; do
    grep -q '0x6996' <<<"$(fn_body "$f" "$tmp/rv.s")" || fail "$f: no parity table on RISC-V"
done
# ---- 5. RISC-V popcount is branch-free -----------------------------------
for f in pc32 pc64; do
    body=$(fn_body "$f" "$tmp/rv.s")
    grep -qE '\bbeq?z|\bbne?z|\bjal|\bj\s|\bcall' <<<"$body" &&
        fail "$f still contains a loop on RISC-V"
    grep -q '0x55555555' <<<"$body" || fail "$f lost the SWAR masks on RISC-V"
done

# Pinned counts for the other backends: the numbers the round was measured at.
pinrv() { local got; got=$(count_of "$1" "$tmp/rv.s"); [ "$got" = "$2" ] || fail "$1: $got instructions on RISC-V, pinned $2"; }
pinrv par32 17
pinrv par64 17
# The narrow-count rule is gated to the back ends whose narrow result needs no
# extension, so these three are the pre-round numbers on both non-x86 targets:
# narrowing here would pay for a mask-and-copy and save nothing.
pinarm() { local got; got=$(count_of "$1" "$tmp/arm.s"); [ "$got" = "$2" ] || fail "$1: $got instructions on AArch64, pinned $2"; }
pinarm par8 14
pinarm pc8 13
pinarm pc16 13
pinrv par8 16
pinrv par16 18
pinrv pc8 24
pinrv pc16 26
pini686() { local got; got=$(count_of "$1" "$tmp/i686.s"); [ "$got" = "$2" ] || fail "$1: $got instructions on i686, pinned $2"; }
# 13: the same 6-instruction algorithm as x86-64, plus the memory-argument
# load, the frame the i686 backend emits and one more copy -- the staging pair
# (`pushl %ecx`/`popl %ecx`) is elided by the scratch-save peephole.
pini686 par32 13
# The narrow shapes on i686: the same rules, the same wins (8 -> 6, 10 -> 7,
# 19 -> 11) plus the two-instruction frame difference the memory-argument
# calling convention adds.
pini686 par8 6
pini686 par16 7
pini686 pc8 11

# ---- 6. Values -----------------------------------------------------------
src=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/parity_and_popcount.c
want=""
for opt in -O0 -O1 -O2 -O3 -Os; do
    "$CCC" "$opt" "$src" -o "$tmp/t" || fail "lccc $opt failed to build the value test"
    out=$("$tmp/t") || fail "lccc $opt: $out"
    case "$out" in *"failures=0 "*) ;; *) fail "lccc $opt: $out" ;; esac
    if [ -z "$want" ]; then
        want=$out
        if command -v gcc >/dev/null 2>&1; then
            gcc -O2 "$src" -o "$tmp/g" && ghash=$("$tmp/g") || ghash=""
            if [ -n "${ghash:-}" ] && [ "$ghash" != "$want" ]; then
                fail "GCC disagrees: $ghash vs $want"
            fi
        fi
    else
        [ "$out" = "$want" ] || fail "opt levels disagree: $out vs $want"
    fi
done
echo "check_parity_and_popcount: OK (parity is a flag/table question on every backend, a byte counts in five instructions, narrow counts run at their own width on x86, popcount is branch-free on RISC-V, values agree with the model at $want)"
