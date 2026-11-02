#!/usr/bin/env bash
# Emission + execution gate for the in-place byte-mask folds (x86-64, and the
# i686 shape they were ported from).
#
# Two rewrites are under test:
#
#   1. `and $255, %R` whose result is only tested -> `testb $mask, %Rb`
#      (one instruction instead of two);
#   2. `and $255, %R` whose value is wanted and whose flags are dead ->
#      `movzbl %Rb, %R32` (four bytes instead of seven).
#
# Both rewrites change which instruction produces EFLAGS, so their failure mode
# is a wrong *branch*, not a build error. That is why this gate is mostly an
# execution gate: every probe is compiled by LCCC and by GCC and their printed
# results must agree bit for bit, at every optimisation level, in both modes.
#
# The discriminating probes are the SIGNED ones. `and`/`test` agree on
# ZF/CF/OF/PF and disagree on SF (`and $255`'s result is at most 255, so its SF
# is always 0), so `(int8_t)x < 0` on x = 0x80..0xFF must stay a sign test. A
# rewrite that only looked at "the mask feeds a branch" would answer
# "not negative" for all of them and this gate would catch it.
#
# The assembly laws are deliberately conditional on the shape being present: the
# emitter may produce the mask already folded into its consumer, and a gate that
# demanded one spelling would fail on a codegen improvement that is not a
# regression here. What is *not* conditional is the negative: a mask that
# survives into a pure test consumer is the defect this fold exists to remove.
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
[[ -x $ccc ]] || {
    echo "check_mask_test_fold: no lccc at $ccc" >&2
    exit 1
}
gcc=${GCC:-gcc}
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-masktest.XXXXXX")
trap 'rm -rf "$work"' EXIT

fail=0
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

cat > "$work/mask.c" <<'EOF'
/* Freestanding output keeps BOTH execution modes independent of host libc
 * headers/start files. It does not shrink the hash or the probe matrix. */
static int print_hash(unsigned long hash) {
    char output[12];
    int n=0;
    hash %= 1000000007ul;
    do { output[n++]=(char)('0'+hash%10); hash/=10; } while(hash);
    for(int a=0,b=n-1;a<b;++a,--b) {
        char t=output[a]; output[a]=output[b]; output[b]=t;
    }
    output[n++]='\n';
#if defined(__i386__)
    long result=4;
    __asm__ volatile("int $0x80" : "+a"(result)
                     : "b"(1), "c"(output), "d"(n) : "memory", "cc");
#else
    long result=1;
    __asm__ volatile("syscall" : "+a"(result)
                     : "D"(1), "S"(output), "d"((long)n)
                     : "rcx", "r11", "memory", "cc");
#endif
    return result!=n;
}

#define NOINLINE __attribute__((noinline))

/* Rule 1: the masked value is only ever tested. */
NOINLINE int nonzero_u8(unsigned x) { return (unsigned char)x != 0; }
NOINLINE int zero_u8(unsigned x) { return (unsigned char)x == 0; }
NOINLINE int bit0_set(unsigned x) { return (x & 1u) != 0; }

/* The discriminating shapes: these must stay SIGN tests. */
NOINLINE int negative_i8(unsigned x) { return (signed char)x < 0; }
NOINLINE int nonneg_i8(unsigned x) { return (signed char)x >= 0; }
NOINLINE int high_bit(unsigned x) { return (unsigned char)x > 127u; }

/* Rule 2: the masked value is wanted; only the flags are dead. */
NOINLINE unsigned keep_u8(unsigned x) { return (unsigned char)x; }
NOINLINE unsigned keep_and_add(unsigned x) { return (unsigned char)x + 1u; }

/* A mask that feeds both a test and a value: the value pins it in place. */
NOINLINE int both_uses(unsigned x) {
    unsigned v = (unsigned char)x;
    return (v != 0) + (int)(v >> 1);
}

int main(void) {
    static const unsigned probes[] = {
        0x00u, 0x01u, 0x7fu, 0x80u, 0x81u, 0xffu, 0x100u, 0x1ffu,
        0xffffff00u, 0xdeadbeefu, 0x0000007fu, 0x80000000u,
    };
    unsigned long hash = 0;
    for (unsigned i = 0; i < sizeof probes / sizeof probes[0]; ++i) {
        unsigned x = probes[i];
        hash = hash * 131u + (unsigned)nonzero_u8(x);
        hash = hash * 131u + (unsigned)zero_u8(x);
        hash = hash * 131u + (unsigned)bit0_set(x);
        hash = hash * 131u + (unsigned)negative_i8(x);
        hash = hash * 131u + (unsigned)nonneg_i8(x);
        hash = hash * 131u + (unsigned)high_bit(x);
        hash = hash * 131u + keep_u8(x);
        hash = hash * 131u + keep_and_add(x);
        hash = hash * 131u + (unsigned)both_uses(x);
    }
    for (unsigned x = 0; x < 512; ++x) {
        hash = hash * 131u + (unsigned)negative_i8(x);
        hash = hash * 131u + (unsigned)nonzero_u8(x);
    }
    return print_hash(hash);
}
EOF

# `-march` is pinned: the driver otherwise defaults to the host CPU, and a
# vectorised build would lower these probes through packed code where neither
# rewrite applies.
"$ccc" -march=x86-64 -O2 -S "$work/mask.c" -o "$work/mask64.s"
"$ccc" -m32 -march=i686 -O2 -S "$work/mask.c" -o "$work/mask32.s"

total_tests=0
for asm in "$work/mask64.s" "$work/mask32.s"; do
    mode=x86-64
    [[ $asm == *32.s ]] && mode=i686
    # Only the two functions whose mask feeds nothing but a condition: their
    # mask must never survive as an `and`. `bit0_set` *returns* `x & 1`, so the
    # mask is the value there and GCC keeps it too (`andl $1, %edi; movl %edi,
    # %eax`); the execution oracle below is what pins that case.
    for fn in nonzero_u8 zero_u8; do
        body "$fn" "$asm" > "$work/one.s"
        # A mask whose only consumer is a condition must not survive, in
        # either spelling of the immediate (the emitter prints decimal today;
        # matching the hex spelling too keeps the law, not the spelling).
        if grep -qE '^[[:space:]]*and[lq]?[[:space:]]+\$(0xff|255),' "$work/one.s"; then
            emit_error "$mode $fn kept a test-only byte mask" "$(cat "$work/one.s")"
            fail=1
        fi
        if grep -qE '^[[:space:]]*and[lq]?[[:space:]]+\$(0x1|1),' "$work/one.s"; then
            emit_error "$mode $fn kept a test-only bit mask" "$(cat "$work/one.s")"
            fail=1
        fi
        if grep -qE '^[[:space:]]*testb' "$work/one.s"; then
            total_tests=$((total_tests + 1))
        fi
    done
    # The sign tests are allowed to keep their mask: an SF reader must refuse.
    for fn in negative_i8 nonneg_i8 high_bit; do
        body "$fn" "$asm" > "$work/one.s"
        if [[ ! -s $work/one.s ]]; then
            emit_error "$mode $fn missing from the assembly" "$fn"
            fail=1
        fi
    done
done
if [[ $total_tests -eq 0 ]]; then
    echo "check_mask_test_fold: note: no 'testb' site observed; the emitter may" \
        "already fold these consumers. Execution checks below remain authoritative." >&2
fi

# ── execution: mandatory GCC oracle, both modes, every level ───────────────
# These are user-space, non-kernel fixtures. No multilib libc or dynamic
# loader is needed; unavailable execution is an error, never a passing skip.
cat > "$work/start64.S" <<'EOF'
.text
.globl _start
_start:
call main
mov %eax,%edi
mov $60,%eax
syscall
.section .note.GNU-stack,"",%progbits
EOF
cat > "$work/start32.S" <<'EOF'
.text
.globl _start
_start:
call main
mov %eax,%ebx
mov $1,%eax
int $0x80
.section .note.GNU-stack,"",%progbits
EOF
run32() {
    if [[ -n ${LCCC_I686_RUNNER:-} ]]; then
        timeout 60 "$LCCC_I686_RUNNER" "$1"
    elif command -v qemu-i386 >/dev/null 2>&1; then
        timeout 60 qemu-i386 "$1"
    else
        timeout 60 "$1"
    fi
}
for mode in 64 32; do
    flags=(-fno-pie -nostdlib -static)
    [[ $mode == 64 ]] || flags+=(-m32)
    timeout 120 "$gcc" "${flags[@]}" -O2 "$work/start$mode.S" "$work/mask.c" -o "$work/ref$mode"
    if [[ $mode == 64 ]]; then
        expected=$(timeout 60 "$work/ref$mode")
    else
        expected=$(run32 "$work/ref$mode")
    fi
    [[ $expected =~ ^[0-9]+$ ]] || { echo "FAIL: invalid $mode-bit oracle output" >&2; exit 1; }
    arch=x86-64
    [[ $mode == 64 ]] || arch=i686
    for opt in -O0 -O1 -O2 -O3 -Os; do
        timeout 120 "$ccc" "${flags[@]}" -march="$arch" "$opt" \
            "$work/start$mode.S" "$work/mask.c" -o "$work/chk$mode"
        if [[ $mode == 64 ]]; then
            actual=$(timeout 60 "$work/chk$mode")
        else
            actual=$(run32 "$work/chk$mode")
        fi
        if [[ $actual != "$expected" ]]; then
            echo "FAIL: $mode-bit $opt -- got '$actual', want '$expected'" >&2
            fail=1
        fi
    done
done
if [[ $fail -ne 0 ]]; then
    echo "check_mask_test_fold: FAILED" >&2
    exit 1
fi
echo "check_mask_test_fold: OK (test-only byte masks folded, sign readers kept," \
    "GCC-identical full hashes at -O0/-O1/-O2/-O3/-Os in mandatory 32- and 64-bit execution)"
