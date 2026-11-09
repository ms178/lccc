#!/usr/bin/env bash
# Emission + execution gate for scratch-save (`push R` / `pop R`) elimination on
# both x86 back ends.
#
# The pass in question deletes a save/restore pair when the span between them
# cannot observe the saved value. That is exactly the kind of transform whose
# failure mode is silent: the emitted code still assembles, still runs, and
# returns a value that is only wrong for the inputs the saved register mattered
# for. So this gate checks BOTH directions on the real compiler:
#
#   * it must DELETE pairs the proof licenses -- on x86-64 the SWAR popcount's
#     scratch save must not survive;
#   * it must KEEP pairs the proof does not license -- the `__lccc_udivdi3`
#     helper saves `%ebx` and reads it before restoring it, and the i686
#     popcount loads its own argument THROUGH the pushed stack slot, so a pass
#     that deleted pairs on shape alone would corrupt 64-bit division and shift
#     that argument load.
#
# And every probe is EXECUTED against GCC's answer for the same source at the
# same optimisation level: an emission check alone cannot see a pair removed
# around a live value.
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
[[ -x $ccc ]] || {
    echo "check_scratch_save_elimination: no lccc at $ccc" >&2
    exit 1
}
gcc=${GCC:-gcc}
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-scratch.XXXXXX")
trap 'rm -rf "$work"' EXIT

fail=0
# Put the diagnosis into the Checks tab. A gate whose only output is a log line
# costs a full CI round trip per iteration, and the run log is not always
# reachable from a review; `%` is escaped first because workflow commands treat
# it as their own escape character.
emit_error() { # <title> <text>
    printf '%s\n' "$2" >&2
    if [[ -n ${GITHUB_ACTIONS:-} ]]; then
        printf '::error title=%s::%s\n' "$1" \
            "$(printf '%s' "$2" | head -c 1200 | tr '\n' ' ' | sed 's/%/%25/g')"
    fi
}
body() { # <function> <asm file> -> that function's body
    sed -n "/^$1:/,/^\.size/p" "$2"
}

cat > "$work/scratch.c" <<'EOF'
/* Header-free on purpose: this fixture is also compiled with -m32 -S on hosts
 * without libc6-dev-i386.  <stdio.h> would pull in glibc's multiarch bits/
 * tree and turn an ASSEMBLER-SHAPE gate into an environment failure.  The
 * printf prototype is identical on LP64 and ILP32. */
extern int printf(const char *, ...);

#define NOINLINE __attribute__((noinline))

/* The canonical SWAR popcount. The accumulator backend stages %ecx through the
 * stack around it; nothing outside the sequence reads %ecx. */
NOINLINE unsigned pop32_swar(unsigned x) {
    x = x - ((x >> 1) & 0x55555555u);
    x = (x & 0x33333333u) + ((x >> 2) & 0x33333333u);
    x = (x + (x >> 4)) & 0x0f0f0f0fu;
    return (x * 0x01010101u) >> 24;
}

/* 64-bit division pulls in the hand-written helper, which saves %ebx and uses
 * it: the pass must leave that pair alone. */
NOINLINE unsigned long long udiv64(unsigned long long a, unsigned long long b) {
    return a / b;
}

NOINLINE unsigned long long urem64(unsigned long long a, unsigned long long b) {
    return a % b;
}

/* A loop whose body keeps a value in a scratch register across iterations. */
NOINLINE unsigned loop_mix(const unsigned *p, unsigned n) {
    unsigned acc = 0;
    for (unsigned i = 0; i < n; ++i) {
        unsigned v = p[i];
        acc += pop32_swar(v) + (v >> 3) + (v << 5);
    }
    return acc;
}

int main(void) {
    unsigned h = 0;
    for (unsigned x = 0; x < 4096; ++x) {
        h = h * 131u + pop32_swar(x);
        h ^= (unsigned)udiv64((unsigned long long)x * 1000003ull, x + 7ull);
        h += (unsigned)urem64((unsigned long long)x * 2654435761ull, x + 11ull);
    }
    static const unsigned arr[16] = {3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5, 8, 9, 7, 9, 3};
    h = h * 131u + loop_mix(arr, 16);
    printf("%u\n", h);
    return 0;
}
EOF

# `-march` is pinned: the driver defaults to the host CPU, and a POPCNT-capable
# host would lower the SWAR sequence to one instruction and hide the very shape
# this gate exists for.
"$ccc" -m32 -march=i686 -O2 -S "$work/scratch.c" -o "$work/scratch32.s"
"$ccc" -march=x86-64 -O2 -S "$work/scratch.c" -o "$work/scratch64.s"

# ── x86-64: the canary pair is gone, and the compiler still builds ──────────
body pop32_swar "$work/scratch64.s" > "$work/pop64.s"
if grep -qE '^[[:space:]]*(pushq|popq)' "$work/pop64.s"; then
    emit_error "x86-64 scratch save not eliminated" "$(cat "$work/pop64.s")"
    fail=1
fi

# ── i686: what the accumulator backend stages, and what it must not ────────
#
# The measured i686 body is
#
#     pop32_swar:
#         pushl %ebx
#         subl $8, %esp
#         movl 16(%esp), %ebx          <- the incoming argument, 4 bytes higher
#         movl %ebx, %eax
#         ... SWAR in %eax/%ecx ...
#         addl $8, %esp
#         popl %ebx
#         ret
#
# Three separate laws are visible in those eight lines, and each one is a way a
# pair-eliminating pass goes wrong:
#
#   * `%ecx` is caller-saved and the sequence uses it freely, so staging it
#     through the stack would be pure waste -- a `pushl %ecx` here is a defect;
#   * `%ebx` IS the saved value, the span WRITES it, and `%ebx` is callee-saved:
#     the pair must survive, and it must survive balanced (a lone `popl` is a
#     stack corruption, not a missed optimisation);
#   * the span loads its argument from `16(%esp)`, i.e. THROUGH the stack slot
#     the push created. Deleting the pair moves that load four bytes off. This
#     is the single most important assertion in the file: it is the `%esp`
#     dependence proof, checked against real compiler output.
body pop32_swar "$work/scratch32.s" > "$work/pop32.s"
if ! grep -qE '^[[:space:]]*(movl|shrl|andl)' "$work/pop32.s"; then
    emit_error "i686 pop32_swar body looks empty" "$(cat "$work/pop32.s")"
    fail=1
fi
if grep -qE '^[[:space:]]*(pushl|popl)[[:space:]]+%ecx' "$work/pop32.s"; then
    emit_error "i686 pop32_swar stages caller-saved %ecx" "$(cat "$work/pop32.s")"
    fail=1
fi
if grep -qE '^[[:space:]]*pushl[[:space:]]+%ebx' "$work/pop32.s"; then
    if ! grep -qE '^[[:space:]]*popl[[:space:]]+%ebx' "$work/pop32.s"; then
        emit_error "i686 pop32_swar has an unbalanced %ebx save" "$(cat "$work/pop32.s")"
        fail=1
    fi
    if ! grep -qE '^[[:space:]]*movl[[:space:]]+16\(%esp\), %ebx' "$work/pop32.s"; then
        emit_error "i686 pop32_swar lost its push-relative argument load" \
            "$(cat "$work/pop32.s")"
        fail=1
    fi
fi

# ── the control: the 64-bit division helper keeps its live %ebx pair ───────
if grep -q '^__lccc_udivdi3:' "$work/scratch32.s"; then
    body __lccc_udivdi3 "$work/scratch32.s" > "$work/udiv.s"
    if ! grep -qE '^[[:space:]]*pushl[[:space:]]+%ebx' "$work/udiv.s"; then
        emit_error "udivdi3 lost its live %ebx save" "$(sed -n '1,14p' "$work/udiv.s")"
        fail=1
    fi
    if ! grep -qE '^[[:space:]]*popl[[:space:]]+%ebx' "$work/udiv.s"; then
        emit_error "udivdi3 lost its %ebx restore (unbalanced stack)" "$(cat "$work/udiv.s")"
        fail=1
    fi
else
    echo "check_scratch_save_elimination: note: no __lccc_udivdi3 in the i686 output" >&2
fi

# ── execution: GCC is the oracle for both modes and every level ────────────
if timeout 120 "$gcc" -O2 "$work/scratch.c" -o "$work/ref64" 2>/dev/null; then
    expected=$(timeout 60 "$work/ref64")
    for opt in -O0 -O1 -O2 -O3 -Os; do
        if ! timeout 120 "$ccc" -march=x86-64 "$opt" "$work/scratch.c" -o "$work/chk64" 2>/dev/null; then
            echo "FAIL: x86-64 build failed at $opt" >&2
            fail=1
            continue
        fi
        actual=$(timeout 60 "$work/chk64")
        if [[ $actual != "$expected" ]]; then
            echo "FAIL: x86-64 $opt -- got '$actual', want '$expected'" >&2
            fail=1
        fi
    done
fi

source "$here/i386_exec.sh"
if i386_run_or_skip "$gcc"; then
    if timeout 120 "$gcc" -m32 -O2 "$work/scratch.c" -o "$work/ref32"; then
        expected32=$(timeout 60 "$work/ref32")
        for opt in -O0 -O1 -O2 -O3 -Os; do
            if ! timeout 120 "$ccc" -m32 -march=i686 "$opt" "$work/scratch.c" \
                -o "$work/chk32" 2>/dev/null; then
                echo "FAIL: i686 build failed at $opt" >&2
                fail=1
                continue
            fi
            actual32=$(timeout 60 "$work/chk32")
            if [[ $actual32 != "$expected32" ]]; then
                echo "FAIL: i686 $opt -- got '$actual32', want '$expected32'" >&2
                fail=1
            fi
        done
    fi
else
    rc=$?
    [[ $rc == 77 ]] || exit "$rc"
fi

if [[ $fail -ne 0 ]]; then
    echo "check_scratch_save_elimination: FAILED" >&2
    exit 1
fi
echo "check_scratch_save_elimination: OK (dead scratch saves removed on both x86 back ends; a live callee-saved pair is preserved; GCC-identical results at -O0/-O1/-O2/-O3/-Os in 32- and 64-bit mode)"
