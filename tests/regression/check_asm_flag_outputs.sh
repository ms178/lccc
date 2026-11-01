#!/usr/bin/env bash
# Flag-output operands (`=@cc<cond>`) are a *per-target* `asm` operand class.
#
# WHY: GCC documents flag outputs for x86, ARM/AArch64 and s390 only, and
# defines `__GCC_ASM_FLAG_OUTPUTS__` exactly on the targets that implement
# them. Two things can go wrong, and both did:
#
#   * a target accepts the constraint but cannot honour it. The AArch64 back end
#     classified `"=@cceq"` and then stored an uninitialised GP register as the
#     operand's value (its own comment said so), and the RISC-V back end mapped
#     `"=@ccz"` to a plain `"r"`. Both compiled cleanly and produced garbage.
#   * a target accepts the *other* target's spelling. `z` names "zero clear" on
#     x86 and is not an AArch64 condition at all (`eq` is); `cs` is the reverse.
#
# The parser now validates the suffix set per target, the AArch64 back end
# materialises the condition with `CSET`, RISC-V refuses the operand, and
# `__GCC_ASM_FLAG_OUTPUTS__` is defined on x86-64/i686/AArch64 only.
#
# Usage: CCC=<x86-64 lccc> [CCC_I686=...] [CCC_ARM=...] [CCC_RISCV=...]
#        [GCC=gcc-16] bash tests/regression/check_asm_flag_outputs.sh
set -uo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
TMP=${TMPDIR:-/tmp}/lccc-flagout.$$
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP"
fail=0

# expect_clean <label> <driver> <source>
expect_clean() {
    local label=$1 driver=$2 src=$3 out
    if ! out=$("$driver" -fsyntax-only "$src" 2>&1); then
        echo "FAIL $label: expected acceptance, got:" >&2
        echo "$out" >&2
        fail=1
    elif [[ -n $out ]]; then
        echo "FAIL $label: expected no diagnostic, got:" >&2
        echo "$out" >&2
        fail=1
    fi
}

# expect_error <label> <driver> <source> <message>
expect_error() {
    local label=$1 driver=$2 src=$3 want=$4 out
    if out=$("$driver" -fsyntax-only "$src" 2>&1); then
        echo "FAIL $label: expected an error, compilation succeeded" >&2
        fail=1
    elif [[ $out != *"$want"* ]]; then
        echo "FAIL $label: expected '$want', got:" >&2
        echo "$out" >&2
        fail=1
    fi
}

# defines <driver> <macro> -> prints "yes"/"no"
defines() {
    local driver=$1 macro=$2
    if "$driver" -dM -E - </dev/null 2>/dev/null | grep -qx "#define $macro 1"; then
        echo yes
    else
        echo no
    fi
}

# ── x86-64: the reference implementation ─────────────────────────────────────
cat >"$TMP/x86-eq.c" <<'EOF'
static __attribute__((noinline)) _Bool eq(int a, int b)
{
    _Bool r;
    __asm__("cmpl %2, %1" : "=@ccz"(r) : "r"(a), "r"(b));
    return r;
}
static __attribute__((noinline)) _Bool lt(int a, int b)
{
    int r;
    __asm__("cmpl %2, %1" : "=@ccl"(r) : "r"(a), "r"(b));
    return r != 0;
}
int main(void)
{
    if (!eq(7, 7) || eq(7, 8))
        return 1;
    if (!lt(1, 2) || lt(2, 1) || lt(5, 5))
        return 2;
    return 0;
}
EOF
for opt in -O0 -O1 -O2 -Os; do
    if ! "$CCC" "$opt" "$TMP/x86-eq.c" -o "$TMP/x86-eq"; then
        echo "FAIL x86-64 $opt: flag outputs must compile" >&2
        fail=1
    elif ! "$TMP/x86-eq"; then
        echo "FAIL x86-64 $opt: flag output produced the wrong boolean" >&2
        fail=1
    fi
done
if [[ $(defines "$CCC" __GCC_ASM_FLAG_OUTPUTS__) != yes ]]; then
    echo "FAIL x86-64: __GCC_ASM_FLAG_OUTPUTS__ must be defined" >&2
    fail=1
fi
# The condition must come from the flags the template left, i.e. a `setcc`
# straight after the template's `cmpl`.
if ! "$CCC" -O2 -S "$TMP/x86-eq.c" -o "$TMP/x86-eq.s"; then
    echo "FAIL x86-64 -S: flag output function did not assemble" >&2
    fail=1
elif ! grep -Eq '^[[:space:]]+sete[[:space:]]+%[a-z0-9]+' "$TMP/x86-eq.s"; then
    echo "FAIL x86-64: no 'sete' materialised the flag output:" >&2
    cat "$TMP/x86-eq.s" >&2
    fail=1
fi
# An AArch64 condition is not an x86 condition.
printf '%s\n' 'int f(int a, int b){ _Bool r; __asm__("cmpl %2,%1" : "=@cceq"(r) : "r"(a),"r"(b)); return r; }' \
    >"$TMP/x86-bad.c"
want="unknown 'asm' flag output '=@cceq'"
expect_error "x86-64 rejects the aarch64 spelling" "$CCC" "$TMP/x86-bad.c" "$want"

# The runtime check above is only meaningful if the oracle agrees the program is
# well-formed, so assert that too when gcc is available.
if command -v "$GCC" >/dev/null 2>&1; then
    if ! "$GCC" -O2 "$TMP/x86-eq.c" -o "$TMP/x86-eq-gcc" 2>"$TMP/gcc.log"; then
        echo "FAIL x86-64: the gcc oracle rejects the reference program:" >&2
        cat "$TMP/gcc.log" >&2
        fail=1
    elif ! "$TMP/x86-eq-gcc"; then
        echo "FAIL x86-64: the gcc-built reference program answers differently" >&2
        fail=1
    fi
fi

# ── every target: the macro must match the implemented support ───────────────
# i686 and AArch64 implement flag outputs; RISC-V has no condition register.
for target in I686 ARM RISCV; do
    eval "driver=\${CCC_$target:-}"
    [[ -z ${driver:-} ]] && continue
    if [[ ! -x $driver ]]; then
        echo "FAIL CCC_$target is set but not executable: $driver" >&2
        fail=1
        continue
    fi
    case $target in
    I686 | ARM)
        if [[ $(defines "$driver" __GCC_ASM_FLAG_OUTPUTS__) != yes ]]; then
            echo "FAIL $target: __GCC_ASM_FLAG_OUTPUTS__ must be defined" >&2
            fail=1
        fi
        ;;
    RISCV)
        if [[ $(defines "$driver" __GCC_ASM_FLAG_OUTPUTS__) != no ]]; then
            echo "FAIL $target: __GCC_ASM_FLAG_OUTPUTS__ must not be defined" >&2
            fail=1
        fi
        ;;
    esac
done

# ── i686: same SETcc materialisation as x86-64 ───────────────────────────────
if [[ -n ${CCC_I686:-} && -x ${CCC_I686} ]]; then
    cat >"$TMP/i686-eq.c" <<'EOF'
int eq(int a, int b)
{
    _Bool r;
    __asm__("cmpl %2, %1" : "=@ccz"(r) : "r"(a), "r"(b));
    return r;
}
EOF
    expect_clean "i686 accepts =@ccz" "$CCC_I686" "$TMP/i686-eq.c"
    if ! "$CCC_I686" -O2 -S "$TMP/i686-eq.c" -o "$TMP/i686-eq.s"; then
        echo "FAIL i686 -S: flag output function did not assemble" >&2
        fail=1
    elif ! grep -Eq '^[[:space:]]+sete[[:space:]]+%[a-z]{2,3}' "$TMP/i686-eq.s"; then
        echo "FAIL i686: no 'sete' materialised the flag output:" >&2
        cat "$TMP/i686-eq.s" >&2
        fail=1
    fi
fi

# ── AArch64: CSET, ARM condition names, x86 names refused ────────────────────
if [[ -n ${CCC_ARM:-} && -x ${CCC_ARM} ]]; then
    cat >"$TMP/arm-cc.c" <<'EOF'
int eq(int a, int b)
{
    _Bool r;
    __asm__("cmp %w1, %w2" : "=@cceq"(r) : "r"(a), "r"(b));
    return r;
}
int ge(int a, int b)
{
    int r;
    __asm__("cmp %w1, %w2" : "=@ccge"(r) : "r"(a), "r"(b));
    return r;
}
EOF
    expect_clean "aarch64 accepts =@cceq/=@ccge" "$CCC_ARM" "$TMP/arm-cc.c"
    if ! "$CCC_ARM" -O2 -S "$TMP/arm-cc.c" -o "$TMP/arm-cc.s"; then
        echo "FAIL aarch64 -S: flag output function did not assemble" >&2
        fail=1
    else
        # The operand's value is the condition, not the register the template
        # left behind: `CSET` must appear, once per flag output.
        n=$(grep -Ec '^[[:space:]]+cset[[:space:]]+w[0-9]+, (eq|ge)\b' "$TMP/arm-cc.s" || true)
        if [[ ${n:-0} -lt 2 ]]; then
            echo "FAIL aarch64: expected a 'cset w<N>, <cond>' per flag output, found ${n:-0}:" >&2
            cat "$TMP/arm-cc.s" >&2
            fail=1
        fi
    fi
    # An x86 condition is not an ARM one.
    printf '%s\n' 'int f(int a,int b){ _Bool r; __asm__("cmp %w1,%w2" : "=@ccz"(r) : "r"(a),"r"(b)); return r; }' \
        >"$TMP/arm-bad.c"
    want="unknown 'asm' flag output '=@ccz'"
    expect_error "aarch64 rejects the x86 spelling" "$CCC_ARM" "$TMP/arm-bad.c" "$want"
fi

# ── RISC-V: no condition register, so the operand is refused ─────────────────
if [[ -n ${CCC_RISCV:-} && -x ${CCC_RISCV} ]]; then
    for spelling in '=@ccz' '=@cceq' '=@cc'; do
        printf 'int f(int a,int b){ _Bool r; __asm__("sub %1, %2" : "%s"(r) : "r"(a),"r"(b)); return r; }\n' \
            "$spelling" >"$TMP/rv-bad.c"
        want="'$spelling' flag output constraint is not supported on this target"
        expect_error "riscv rejects $spelling" "$CCC_RISCV" "$TMP/rv-bad.c" "$want"
    done
fi

# ── byte and high-byte views: the register file is not uniform ───────────────
# `%b<n>` prints the low byte of the operand and `%h<n>` the high byte. On
# x86-64 every register has a low byte (sil/dil/bpl/spl and r8b-r15b are REX
# encodings), but only rax/rbx/rcx/rdx have a high byte; on IA-32 only
# al/bl/cl/dl are byte-addressable at all. Both back ends used to ignore the
# modifier and hand out any register of the general pool, which printed a name
# the assembler rejects (`movb %esi`) or, worse, a *low* byte where the template
# asked for the high one (`%h` on rsi printed %sil).
cat >"$TMP/x86-hi.c" <<'EOF'
int hi(void)
{
    int a, b, c;
    __asm__("movl $1, %0\n\tmovl $2, %1\n\tmovb $3, %h2"
            : "=r"(a), "=r"(b), "=r"(c));
    return a + b + c;
}
EOF
if ! "$CCC" -O2 -S "$TMP/x86-hi.c" -o "$TMP/x86-hi.s"; then
    echo "FAIL x86-64 -S: %h operand function did not assemble" >&2
    fail=1
elif ! grep -Eq '^[[:space:]]+movb \$3, %(ah|bh|ch|dh)\b' "$TMP/x86-hi.s"; then
    echo "FAIL x86-64: %h2 was not materialised through a real high byte:" >&2
    cat "$TMP/x86-hi.s" >&2
    fail=1
fi

if [[ -n ${CCC_I686:-} && -x ${CCC_I686} ]]; then
    # Three byte outputs: the third is the one the general pool used to answer
    # with `esi`, which has no byte name on IA-32. Four is the encoding limit,
    # so three exercises the allocator without exhausting the class.
    cat >"$TMP/i686-byte.c" <<'EOF'
int bytes(void)
{
    unsigned char r0, r1, r2;
    __asm__("movb $1, %b0\n\tmovb $2, %b1\n\tmovb $3, %b2"
            : "=q"(r0), "=q"(r1), "=q"(r2));
    return r0 + r1 + r2;
}
EOF
    expect_clean "i686 accepts three %b operands" "$CCC_I686" "$TMP/i686-byte.c"
    if ! "$CCC_I686" -O2 -S "$TMP/i686-byte.c" -o "$TMP/i686-byte.s"; then
        echo "FAIL i686 -S: %b operand function did not assemble" >&2
        fail=1
    elif grep -E 'movb' "$TMP/i686-byte.s" | grep -Eq '%(esi|edi|ebp)\b'; then
        echo "FAIL i686: a %b operand was given a register with no byte view:" >&2
        cat "$TMP/i686-byte.s" >&2
        fail=1
    elif ! grep -Eq 'movb \$3, %(al|bl|cl|dl)\b' "$TMP/i686-byte.s"; then
        echo "FAIL i686: the third %b operand did not use a byte register:" >&2
        cat "$TMP/i686-byte.s" >&2
        fail=1
    fi
fi

if ((fail)); then
    echo "check_asm_flag_outputs: FAILED" >&2
    exit 1
fi
echo "check_asm_flag_outputs: PASSED"
