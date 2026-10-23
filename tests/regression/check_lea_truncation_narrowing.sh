#!/usr/bin/env bash
# Gate for `lea_narrow` (`narrow_lea_truncations`).
#
# The fold rewrites a 64-bit address computation that is immediately truncated
# into the 32-bit form that already produces the truncated value:
#
#     leaq  8(%rdi), %rdx          leal  8(%rdi), %r13d
#     ...                 ==>      ...
#     movl  %edx, %r13d            (deleted)
#
# x86 address arithmetic wraps modulo 2^64 and the low 32 bits of a sum depend
# only on the low 32 bits of its inputs, so the `leal` computes exactly the
# value the `movl` truncation produced -- and zero-extends it into the 64-bit
# destination the same way.  This is the shape every `(uint32_t)(p + k)` cast
# of an address-scale value compiles to.
#
# SCOPE, STATED HONESTLY
# ----------------------
# It fires on 2 of the 49 benchmark programs (byte_load_reassembly: 163 -> 161
# instructions, glibc_strstr: 173 -> 172) and removes two of the sixteen
# instructions in byte_load_reassembly's decode loop -- the corpus's worst
# remaining ratio against GCC (1.507x at the time of writing).
#
# MEASURED EFFECT
# ---------------
# Paired on the repository runner (`run_benchmarks.py --compilers lccc
# --candidate base=<same tree without the pass>`, -O2, 9 reps, 2 warm-ups):
#   byte_load_reassembly  5.515 ms -> 5.339 ms   (-3.2%, CV 15-17%)
#   geomean of 15 arms    0.9967                 (inside the noise floor)
# The 12 arms whose generated code cannot change bound that noise floor at
# +-1..2% (hash_table +2.2%, sqlite_varint +2.2% on unchanged code), so the
# shrink is directional evidence for the loop it rewrites, not a corpus-level
# speed claim.  This is a codegen-fidelity fold of the same family as
# `zero_ext_cmp`, and the gate asserts exactly that.
#
# WHY A GATE AT ALL FOR SOMETHING THIS SMALL
# ------------------------------------------
# Because the failure direction is a silent miscompile, not a slow program.  The
# fold deletes a `movl` and moves the destination's write EARLIER (back to the
# `leaq`), so it is only correct when:
#   * nothing between the two lines mentions the `leaq` destination (an earlier
#     use would read the value the deleted line produced);
#   * nothing between the two lines mentions the `movl` destination (a read
#     would see the widened value move up; a write would clobber it);
#   * no label, branch, call, return or opaque directive sits between them (a
#     jump into the deleted line changes what the branch executes, and a call
#     reads and writes caller-saved registers without naming them);
#   * the 64-bit destination is not live after the truncation;
#   * the address carries no relocation (`sym@TPOFF`, `%rip`, `%fs:`), because
#     the assembler derives the relocation kind from the `lea` operand size.
#
# Asserted:
#   (1) APPLIES on real corpus code -- byte_load_reassembly must still shrink
#       against `CCC_PEEPHOLE_SKIP=lea_narrow`, and the shape that disappears
#       must be the `leaq`+`movl` pair (a `leal` appears, a `leaq` and a `movl`
#       go away) rather than "something changed".
#   (2) THE SWITCH IS REAL -- the differential must show a difference; if the
#       skip stopped gating the fold, (1) would pass by comparing two identical
#       builds.
#   (3) THE RELOCATION LAW, on real code -- `tls_seg_access`'s
#       `leaq tls_slots@TPOFF(%rax), %rdx` must stay 64-bit.  Narrowing it would
#       change `R_X86_64_TPOFF64` into `R_X86_64_TPOFF32`, which is the linker's
#       contract and not a peephole's.
#   (4) EXECUTION -- every program this gate compiles runs, and its stdout is
#       compared against the GCC oracle at -O0, -O1, -O2 and -O3.  The corpus
#       program prints its own checksum over 20000 rounds of the loop the fold
#       rewrites, so a shape-correct but semantically wrong rewrite cannot pass
#       on asm alone.
#   (5) EXECUTION, live-wide-use fixture -- a kernel whose lea result is read at
#       64-bit width after the truncation must produce the same answer as GCC.
#       The pass refuses that shape; this pins the refusal where a wrong fold
#       would change the result, not just the line count.
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo_root"

LCCC=${LCCC:-${CCC:-target/fastbuild/lccc}}
[[ "$LCCC" == /* ]] || LCCC="$repo_root/$LCCC"
[[ -x "$LCCC" ]] || { echo "FAIL: lccc not found at $LCCC" >&2; exit 1; }

GCC=${GCC:-gcc}
command -v "$GCC" >/dev/null 2>&1 || { echo "FAIL: oracle gcc not found ($GCC)" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fails=0
bad() { echo "FAIL: $*" >&2; fails=$((fails + 1)); }
ok() { echo "ok: $*"; }

insns() { grep -cE '^[[:space:]]+[a-z]' "$1" || true; }
# Count `leaq` / `leal` instructions whose address is a plain register+offset.
count_lea() { grep -cE "^[[:space:]]+$1[[:space:]]+[-0-9x]*(%\\(|\\()" "$2" || true; }

# ── (1)+(2) the fold must still fire, and only the pair may disappear ───
src=tests/benchmark/programs/byte_load_reassembly.c
if [ ! -f "$src" ]; then
    bad "byte_load_reassembly.c missing"
else
    "$LCCC" -O2 -S -o "$work/on.s" "$src" 2>/dev/null || bad "byte_load_reassembly: compile failed"
    CCC_PEEPHOLE_SKIP=lea_narrow "$LCCC" -O2 -S -o "$work/off.s" "$src" 2>/dev/null ||
        bad "byte_load_reassembly: compile with the skip failed"
    if [ -s "$work/on.s" ] && [ -s "$work/off.s" ]; then
        if cmp -s "$work/on.s" "$work/off.s"; then
            bad "byte_load_reassembly: fold no longer fires (skip-on and skip-off asm are identical)"
        else
            on=$(insns "$work/on.s")
            off=$(insns "$work/off.s")
            if [ "$on" -lt "$off" ]; then
                ok "byte_load_reassembly: fold fires, $off -> $on instructions"
            else
                bad "byte_load_reassembly: fold changed the asm but did not shrink it ($off -> $on)"
            fi
            # The fold is exactly "one leaq and one movl become one leal", so the
            # `leaq` census must drop and the `leal` census must rise by the same
            # amount -- this is what makes (1) a pin on the shape and not on
            # "the text changed".
            on_q=$(count_lea 'leaq' "$work/on.s")
            off_q=$(count_lea 'leaq' "$work/off.s")
            on_l=$(count_lea 'leal' "$work/on.s")
            off_l=$(count_lea 'leal' "$work/off.s")
            if [ "$on_q" -lt "$off_q" ] && [ "$on_l" -gt "$off_l" ] &&
                [ $((off_q - on_q)) -eq $((on_l - off_l)) ]; then
                ok "byte_load_reassembly: shape is the leaq->leal narrowing (leaq $off_q->$on_q, leal $off_l->$on_l)"
            else
                bad "byte_load_reassembly: expected one leaq to become one leal (leaq $off_q->$on_q, leal $off_l->$on_l)"
            fi
        fi
    fi
fi

# ── (3) the relocation law, on real TLS code ───────────────────────────
tls=tests/benchmark/programs/tls_seg_access.c
if [ ! -f "$tls" ]; then
    bad "tls_seg_access.c missing"
else
    "$LCCC" -O2 -S -o "$work/tls.s" "$tls" 2>/dev/null || bad "tls_seg_access: compile failed"
    if [ -s "$work/tls.s" ]; then
        if grep -qE 'leaq[[:space:]]+[A-Za-z_][A-Za-z0-9_]*@TPOFF\(' "$work/tls.s"; then
            ok "tls_seg_access: TPOFF address keeps its 64-bit leaq (relocation shape preserved)"
        else
            bad "tls_seg_access: no 64-bit TPOFF leaq left -- a relocation-carrying address was narrowed"
        fi
        if grep -qE 'leal[[:space:]]+[A-Za-z_][A-Za-z0-9_]*@' "$work/tls.s"; then
            bad "tls_seg_access: a leal carries a relocation suffix"
        fi
    fi
fi

# ── (4)+(5) execution differential against GCC, every opt level ────────
cat > "$work/live_wide.c" <<'EOF'
#include <stdio.h>
typedef unsigned int u32;
typedef unsigned long long u64;
/* Every kernel below cancels the object's own address before returning: a
   truncated absolute address is NOT comparable across compilers, because the
   two binaries link the array at different addresses.  Cancelling it keeps the
   `lea`+truncate shape (which is what the fold rewrites) and makes the answer
   depend only on the offset. */
/* The lea result is truncated AND read at 64-bit width: the fold must refuse,
   and the answer must stay correct if it ever stops refusing. */
__attribute__((noinline)) u64 wide(const u32 *p, unsigned i)
{
    unsigned long base = (unsigned long)p;
    unsigned long q = base + (unsigned long)(i + 8) * sizeof *p;
    u64 w = (u64)q - (u64)base;
    u32 d = (u32)q - (u32)base;
    return w + d;
}
/* Pure truncation, the shape the fold exists for. */
__attribute__((noinline)) u32 narrow(const u32 *p, unsigned i)
{
    u32 base = (u32)(unsigned long)p;
    u32 d = (u32)(unsigned long)(p + i + 8) - base;
    return d + 3u;
}
/* Truncation with independent work in between. */
__attribute__((noinline)) u32 interleaved(const u32 *p, unsigned i, u32 s)
{
    u32 base = (u32)(unsigned long)p;
    u32 a = (u32)(unsigned long)(p + i + 1) - base;
    u32 b = (u32)(unsigned long)(p + i + 2) - base;
    u32 c = s ^ (u32)i;
    return a + b + c;
}
int main(void)
{
    static u32 buf[64];
    u64 h = 0;
    unsigned i;
    for (i = 0; i < 64; i++)
        buf[i] = (u32)(i * 2654435761u);
    for (i = 0; i + 16 < 64; i++) {
        /* Sequenced explicitly: reading `h` while assigning it in the same
           full expression would be undefined and both arms would be free to
           disagree for reasons that have nothing to do with the fold. */
        u64 t = wide(buf, i) + narrow(buf, i) + interleaved(buf, i, (u32)h);
        h = h * 31u + t;
    }
    printf("live-wide: %016llx\n", (unsigned long long)h);
    return 0;
}
EOF

run_pair() { # run_pair <label> <src> <opt>
    local label=$1 src=$2 opt=$3
    if ! "$GCC" "$opt" "$src" -o "$work/g" 2>/dev/null; then
        bad "$label $opt: gcc oracle build failed"
        return 0
    fi
    if ! "$LCCC" "$opt" "$src" -o "$work/l" 2>/dev/null; then
        bad "$label $opt: lccc build failed"
        return 0
    fi
    local g l
    g=$("$work/g" 2>&1)
    l=$("$work/l" 2>&1)
    if [ "$g" = "$l" ]; then
        ok "$label $opt: matches gcc ($g)"
    else
        bad "$label $opt: lccc [$l] != gcc [$g]"
    fi
}

for opt in -O0 -O1 -O2 -O3; do
    run_pair byte_load_reassembly "$src" "$opt"
    run_pair tls_seg_access "$tls" "$opt"
    run_pair live_wide "$work/live_wide.c" "$opt"
done

if [ "$fails" -ne 0 ]; then
    echo "lea truncation narrowing: $fails FAIL(S)" >&2
    exit 1
fi
echo "ok: lea truncation narrowing (fires on the real corpus loop, switch is real, relocation shape preserved, matches gcc at -O0..-O3)"
