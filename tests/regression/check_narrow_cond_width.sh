#!/usr/bin/env bash
# Condition tests on register-resident values must be sized to the condition's
# IR width — `testq` on a 32-bit condition reads the ABI-undefined upper
# register bits and can mis-flip ZF (select/branch the wrong arm) when the
# caller leaves garbage above bit 31. The backend sizes the in-place test to
# the value type: testl/testw/testb for ≤32-bit conditions, testq only for
# 64-bit ones. This check asserts the shapes AND drives a genuine dirty
# caller (SysV: upper bits of a narrow integer argument are undefined) against
# the lccc-built function.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

cat >"$td/w.c" <<'EOF'
int s32(unsigned c, int a, int b) { return c ? a : b; }
int fbr(unsigned c) { int r = 0; if (c) r += 3; if (c > 3u) r += 5; return r; }
long s64(unsigned long c, long a, long b) { return c ? a : b; }
int sel(unsigned c, int a, int b) { return c ? a : b; }
EOF

"$CCC" -O3 -S "$td/w.c" -o "$td/w.s"

function_body() {
    local function=$1 file=$2
    awk -v wanted="$function" '
        $0 == wanted ":" { active=1 }
        active { print }
        active && $0 == ".size " wanted ", .-" wanted { exit }
    ' "$file"
}

# 32-bit condition selects/branches must test 32 bits (never testq): every
# condition in s32/fbr is 32-bit, so a full-width testq in the body is the bug.
for fn in s32 fbr; do
    body=$(function_body "$fn" "$td/w.s")
    grep -Eq '^[[:space:]]+testl[[:space:]]+%[a-z0-9]+, %[a-z0-9]+' <<<"$body" || {
        echo "FAIL: $fn lost its width-correct testl" >&2
        printf '%s\n' "$body" >&2
        exit 1
    }
    if grep -Eq '^[[:space:]]+testq[[:space:]]+%[a-z0-9]+, %[a-z0-9]+' <<<"$body"; then
        echo "FAIL: $fn still testq's a 32-bit condition" >&2
        printf '%s\n' "$body" >&2
        exit 1
    fi
done

# 64-bit conditions keep the full-width testq.
body=$(function_body s64 "$td/w.s")
grep -Eq '^[[:space:]]+testq[[:space:]]+%[a-z0-9]+, %[a-z0-9]+' <<<"$body" || {
    echo "FAIL: s64 lost its full-width testq" >&2
    printf '%s\n' "$body" >&2
    exit 1
}
if grep -Eq '^[[:space:]]+test[lwb][[:space:]]+' <<<"$body"; then
    echo "FAIL: s64 narrowed its 64-bit condition test" >&2
    printf '%s\n' "$body" >&2
    exit 1
fi

# Runtime proof with a genuine dirty caller: %rdi low 32 bits are 0 (so
# c == 0 → sel must return b = 222), bit 32 is garbage. A testq would read
# the garbage, see "nonzero", and wrongly return a = 111.
cat >"$td/driver.c" <<'DRIVER'
extern int sel(unsigned c, int a, int b);
int main(void) {
    int r;
    __asm__ volatile(
        "movabsq $0x100000000, %%rdi\n\t"
        "movl $111, %%esi\n\t"
        "movl $222, %%edx\n\t"
        "call sel\n\t"
        "movl %%eax, %0\n\t"
        : "=r"(r)
        : : "rdi", "rsi", "rdx", "rax", "rcx", "r11", "cc", "memory");
    return r == 222 ? 0 : 1;
}
DRIVER
"$CCC" -O3 -c "$td/w.c" -o "$td/w.o"
if command -v cc >/dev/null 2>&1; then
    cc "$td/driver.c" "$td/w.o" -o "$td/dirty"
    "$td/dirty" || {
        echo "FAIL: dirty-caller select returned the true arm for a zero condition" >&2
        exit 1
    }
else
    echo "note: no host cc — dirty-caller runtime proof skipped" >&2
fi

echo "OK narrow_cond_width"
