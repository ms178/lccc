#!/usr/bin/env bash
# A 32-bit logical right shift by 24 followed by sign-extension of its low
# byte is exactly an arithmetic right shift by 24.  With a dead shift
# temporary and unobservable flags, fold the copy/shift/movsbl sequence to
# copy/sarl.  The skip arm is the structural and runtime control.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
CCC=$(readlink -f "$CCC")
GCC=$(readlink -f "$(command -v "$GCC")")
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
src=tests/benchmark/programs/switch_lut_bool_bench.c
flags=(-DN=250000)

"$CCC" -O2 -S "${flags[@]}" "$src" -o "$td/normal.s"
CCC_PEEPHOLE_SKIP=shift_signext "$CCC" -O2 -S "${flags[@]}" "$src" -o "$td/skip.s"
normal=$(sed -n '/^main:/,/^\.size[[:space:]]*main/p' "$td/normal.s")
skip=$(sed -n '/^main:/,/^\.size[[:space:]]*main/p' "$td/skip.s")

if ! grep -qE 'sarl[[:space:]]+\$24,' <<<"$normal" ||
    grep -qE 'shrl[[:space:]]+\$24,|movsbl' <<<"$normal"; then
    echo "optimized main must contain sarl \$24 and no shrl/movsbl remnant:" >&2
    echo "$normal" >&2
    exit 1
fi
if ! grep -qE 'shrl[[:space:]]+\$24,' <<<"$skip" ||
    ! grep -q 'movsbl' <<<"$skip"; then
    echo "shift_signext skip arm must retain the original shrl/movsbl sequence:" >&2
    echo "$skip" >&2
    exit 1
fi

"$CCC" -O2 "${flags[@]}" "$src" -o "$td/normal"
CCC_PEEPHOLE_SKIP=shift_signext "$CCC" -O2 "${flags[@]}" "$src" -o "$td/skip"
normal_out=$("$td/normal")
skip_out=$("$td/skip")
[ "$normal_out" = "$skip_out" ] || {
    echo "normal/skip output divergence: $normal_out vs $skip_out" >&2
    exit 1
}

# The vowel benchmark alone cannot distinguish negative sign-extended bytes
# from large positive ones: both take its default arm.  Check the actual
# signed result for every high byte through an external call, with the source
# kept live by a volatile store after that call.  The provider is compiled by
# GCC so LCCC cannot infer or rewrite the callee's value contract.
cat >"$td/provider.c" <<'EOF'
__attribute__((noinline)) int consume_shifted(int value) { return value; }
EOF
cat >"$td/values.c" <<'EOF'
#include <stdio.h>

extern int consume_shifted(int value);
volatile unsigned shifted_source;

__attribute__((noinline)) static int shifted(unsigned value) {
    int result = consume_shifted((signed char)(value >> 24));
    shifted_source = value;
    return result;
}

int main(void) {
    static const unsigned low[] = {0, 1, 0x123456, 0x7fffff, 0x800000, 0xffffff};
    unsigned long long checksum = 0;
    for (unsigned high = 0; high < 256; ++high) {
        for (unsigned j = 0; j < sizeof(low) / sizeof(low[0]); ++j) {
            unsigned value = (high << 24) | low[j];
            int got = shifted(value);
            int want = (signed char)(value >> 24);
            if (got != want || shifted_source != value) {
                printf("shift mismatch: %08x got %d want %d\\n", value, got, want);
                return 1;
            }
            checksum = checksum * 257 + (unsigned)got;
        }
    }
    printf("shift-values: %llu\\n", checksum);
    return 0;
}
EOF

"$GCC" -O2 -c "$td/provider.c" -o "$td/provider.o"
"$CCC" -O2 -S "$td/values.c" -o "$td/values-normal.s"
CCC_PEEPHOLE_SKIP=shift_signext "$CCC" -O2 -S "$td/values.c" -o "$td/values-skip.s"
value_normal=$(sed -n '/^shifted:/,/^\.size[[:space:]]*shifted/p' "$td/values-normal.s")
value_skip=$(sed -n '/^shifted:/,/^\.size[[:space:]]*shifted/p' "$td/values-skip.s")
if ! grep -qE 'sarl[[:space:]]+\$24,' <<<"$value_normal" ||
    grep -qE 'shrl[[:space:]]+\$24,|movsbl' <<<"$value_normal" ||
    ! grep -qE 'shrl[[:space:]]+\$24,' <<<"$value_skip" ||
    ! grep -q 'movsbl' <<<"$value_skip"; then
    echo "signed-value reproducer lost its normal/skip structural contract" >&2
    echo "$value_normal" >&2
    echo "$value_skip" >&2
    exit 1
fi
"$CCC" -O2 "$td/values.c" "$td/provider.o" -o "$td/values-normal"
CCC_PEEPHOLE_SKIP=shift_signext "$CCC" -O2 "$td/values.c" "$td/provider.o" -o "$td/values-skip"
"$GCC" -O2 "$td/values.c" "$td/provider.o" -o "$td/values-gcc"
value_normal_out=$("$td/values-normal")
value_skip_out=$("$td/values-skip")
value_gcc_out=$("$td/values-gcc")
[ "$value_normal_out" = "$value_skip_out" ] &&
    [ "$value_normal_out" = "$value_gcc_out" ] || {
    echo "signed-value normal/skip/GCC divergence: $value_normal_out / $value_skip_out / $value_gcc_out" >&2
    exit 1
}

for opt in -O0 -O1 -O2 -O3 -Os; do
    "$CCC" "$opt" "${flags[@]}" "$src" -o "$td/lccc"
    "$GCC" "$opt" "${flags[@]}" "$src" -o "$td/gcc"
    lccc_out=$("$td/lccc")
    gcc_out=$("$td/gcc")
    [ "$lccc_out" = "$gcc_out" ] || {
        echo "$opt LCCC/GCC output divergence: $lccc_out vs $gcc_out" >&2
        exit 1
    }
done

echo "OK shift_signext_fold (sarl structure, 256 signed high bytes, skip control, O0..Os GCC parity)"
