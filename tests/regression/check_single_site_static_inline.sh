#!/usr/bin/env bash
# Single-call-site `static inline` helpers must be inlined even when they
# exceed the normal loop-callee size limits (GCC -finline-functions-called-once
# parity). Inlining the only call is always a net code shrink: the outlined
# body, prologue/epilogue and call sequence all disappear.
#
# Motivating case: zstd_count (10 blocks, 63 IR instructions, two loops, one
# hot call site) stayed outlined behind 2M calls because the single-call-site
# exemption excluded `static inline` and the exceeds-normal-limits gate had no
# dead-after-inline exemption.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-single-site-inline.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

# --- Positive: loop-containing static inline helper, one call site ---------
cat >"$tmp/t.c" <<'C'
static unsigned long buf[1024];

static inline unsigned int scan_count(const unsigned long *p,
                                      const unsigned long *q,
                                      const unsigned long *limit) {
    const unsigned long *start = p;
    const unsigned long *loop_limit = limit - 1;
    while (p < loop_limit) {
        unsigned long diff = *p ^ *q;
        if (diff != 0UL) {
            unsigned int bit = 0U;
            while ((diff & 1UL) == 0UL) {
                diff >>= 1;
                bit++;
            }
            p += bit / 8U;
            return (unsigned int)(p - start);
        }
        p++;
        q++;
    }
    while ((p < limit) && (*p == *q)) {
        p++;
        q++;
    }
    return (unsigned int)(p - start);
}

int main(void) {
    unsigned long long checksum = 0ULL;
    unsigned int pass, i;
    for (i = 0; i < 1024; i++)
        buf[i] = (unsigned long)i * 0x9E3779B97F4A7C15ULL;
    for (pass = 0; pass < 64; pass++) {
        for (i = 0; i < 256; i++) {
            unsigned int off1 = (i * 37U + pass) & 511U;
            unsigned int off2 = (i * 53U + pass * 3U) & 511U;
            unsigned int n = scan_count(buf + off1, buf + off2,
                                        buf + off1 + 64);
            checksum += (unsigned long long)n * (i + 1U);
        }
    }
    /* checksum printed so the binary's result is verifiable. */
    __builtin_printf("%llu\n", checksum);
    return 0;
}
C
"$CCC" -O2 -S "$tmp/t.c" -o "$tmp/t.s"
"$CCC" -O2 "$tmp/t.c" -o "$tmp/t"
"$tmp/t" >"$tmp/got.txt"
gcc -O2 "$tmp/t.c" -o "$tmp/ref" 2>/dev/null && "$tmp/ref" >"$tmp/want.txt" || {
    echo "SKIP: host gcc unavailable for oracle" >&2
    exit 0
}
if ! diff -q "$tmp/got.txt" "$tmp/want.txt" >/dev/null; then
    echo "single-site inline: runtime mismatch vs gcc oracle" >&2
    diff "$tmp/got.txt" "$tmp/want.txt" >&2 || true
    exit 1
fi
main_body=$(sed -n '/^main:/,/^\.size main/p' "$tmp/t.s")
if grep -Eq 'call[[:space:]]+scan_count' <<<"$main_body"; then
    echo "single-site inline: hot 'call scan_count' survived in main" >&2
    grep -n "call" "$tmp/t.s" >&2 || true
    exit 1
fi

# --- Negative: address-taken helper must stay outlined ---------------------
cat >"$tmp/addr.c" <<'C'
static unsigned long buf[64];

static inline unsigned int scan_count(const unsigned long *p,
                                      const unsigned long *q,
                                      const unsigned long *limit) {
    const unsigned long *start = p;
    while (p < limit) {
        unsigned long diff = *p ^ *q;
        if (diff != 0UL) {
            while ((diff & 1UL) == 0UL)
                diff >>= 1;
            return (unsigned int)(p - start) + 1U;
        }
        p++;
        q++;
    }
    return (unsigned int)(p - start);
}
typedef unsigned int (*scan_fn)(const unsigned long *,
                                const unsigned long *,
                                const unsigned long *);
volatile scan_fn escaping;
int main(void) {
    unsigned int i;
    unsigned long long checksum = 0ULL;
    for (i = 0; i < 64; i++)
        buf[i] = (unsigned long)i * 3UL + 1UL;
    escaping = scan_count;
    for (i = 0; i < 64; i++)
        checksum += escaping(buf, buf + 1, buf + 8);
    __builtin_printf("%llu\n", checksum);
    return 0;
}
C
"$CCC" -O2 -S "$tmp/addr.c" -o "$tmp/addr.s"
"$CCC" -O2 "$tmp/addr.c" -o "$tmp/addr"
"$tmp/addr" >"$tmp/addr_got.txt"
"$tmp/ref" >/dev/null 2>&1 || true
gcc -O2 "$tmp/addr.c" -o "$tmp/addr_ref" && "$tmp/addr_ref" >"$tmp/addr_want.txt"
if ! diff -q "$tmp/addr_got.txt" "$tmp/addr_want.txt" >/dev/null; then
    echo "single-site inline: address-taken runtime mismatch" >&2
    exit 1
fi
# The address-taken body must survive as an outlined symbol (the volatile
# function pointer calls it indirectly).
if ! grep -Eq '^scan_count:' "$tmp/addr.s"; then
    echo "single-site inline: address-taken body wrongly deleted" >&2
    exit 1
fi
echo "single-site static-inline: OK (hot call inlined, address-taken kept)"
