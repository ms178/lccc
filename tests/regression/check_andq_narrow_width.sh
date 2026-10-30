#!/usr/bin/env bash
# A nonnegative imm32 ANDQ mask sign-extends to a mask with a zero upper
# half; ANDL independently zero-extends its dword result. In particular,
# 0x80..0xff proves an unsigned-byte value for either opcode: the low-byte
# sign bit is irrelevant to zero-extension. The 0x100 control must keep
# movzbl because its un-narrowed value can be 0x100.
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
[[ -x $CCC ]] || { echo "check_andq_narrow_width: no lccc at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_andq_narrow_width: no GCC oracle" >&2; exit 1; }
CCC=$(readlink -f "$CCC")
GCC=$(readlink -f "$(command -v "$GCC")")
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-andq-width.XXXXXX")
trap 'rm -rf "$work"' EXIT

cat >"$work/probe.c" <<'EOF'
#include <stdio.h>
typedef unsigned long u64;
__attribute__((noinline)) u64 f127(u64 x) { return (unsigned char)(x & 127); }
__attribute__((noinline)) u64 f128(u64 x) { return (unsigned char)(x & 128); }
__attribute__((noinline)) u64 f254(u64 x) { return (unsigned char)(x & 254); }
// An earlier canonical fold turns this into a direct byte move; keep it as
// the 0xff runtime boundary, not as an ANDQ structural probe.
__attribute__((noinline)) u64 f255(u64 x) { return (unsigned char)(x & 255); }
__attribute__((noinline)) u64 f256(u64 x) { return (unsigned char)(x & 256); }
int main(void) {
    static const u64 values[] = {
        0, 1, 127, 128, 129, 254, 255, 256, 511,
        0x8000000000000000UL, 0xffffffffffffffffUL,
        0x123456789abcdef0UL, 0x8080808080808080UL
    };
    u64 checksum = 0;
    for (unsigned i = 0; i < sizeof(values) / sizeof(values[0]); ++i) {
        u64 x = values[i];
        checksum = checksum * 131 + f127(x);
        checksum = checksum * 131 + f128(x);
        checksum = checksum * 131 + f254(x);
        checksum = checksum * 131 + f255(x);
        checksum = checksum * 131 + f256(x);
    }
    printf("andq-width: %lu\n", checksum);
    return 0;
}
EOF

body() {
    awk -v sym="$2:" '$0 == sym { inside=1 } inside { print } inside && /^\.size[[:space:]]/ { exit }' "$1"
}
env -u CCC_NO_REDUNDANT_EXT "$CCC" -O2 -S "$work/probe.c" -o "$work/on.s"
CCC_NO_REDUNDANT_EXT=1 "$CCC" -O2 -S "$work/probe.c" -o "$work/off.s"
for fn in f127 f128 f254; do
    on=$(body "$work/on.s" "$fn")
    off=$(body "$work/off.s" "$fn")
    if grep -q 'movzbl' <<<"$on" || ! grep -qE 'and[ql][[:space:]]' <<<"$on"; then
        echo "$fn: redundant byte re-extension survived or AND shape vanished" >&2
        echo "$on" >&2
        exit 1
    fi
    # CCC_NO_REDUNDANT_EXT is a runtime control, not an assembly oracle:
    # independent relay/return passes may legally select movl there too.
    # Reverting the 0xff producer itself is caught by the `on` assertion.
    grep -qE 'and[ql][[:space:]]' <<<"$off" || {
        echo "$fn: kill-switch control lost the intended AND input shape" >&2
        echo "$off" >&2
        exit 1
    }
done
# 0x100 is not a byte-range proof, even though it is a valid positive imm32.
on256=$(body "$work/on.s" f256)
grep -q 'movzbl' <<<"$on256" || {
    echo "f256: byte re-extension was deleted without a byte-range proof" >&2
    echo "$on256" >&2
    exit 1
}

for opt in -O0 -O1 -O2 -O3 -Os; do
    "$GCC" "$opt" "$work/probe.c" -o "$work/ref"
    env -u CCC_NO_REDUNDANT_EXT "$CCC" "$opt" "$work/probe.c" -o "$work/on"
    CCC_NO_REDUNDANT_EXT=1 "$CCC" "$opt" "$work/probe.c" -o "$work/off"
    "$work/ref" >"$work/ref.out"
    "$work/on" >"$work/on.out"
    "$work/off" >"$work/off.out"
    cmp "$work/ref.out" "$work/on.out"
    cmp "$work/ref.out" "$work/off.out"
done

echo "OK andq_narrow_width (0x7f/0x80/0xfe ANDQ/ANDL folds, 0xff canonical runtime, 0x100 refusal, 5-level GCC parity)"
