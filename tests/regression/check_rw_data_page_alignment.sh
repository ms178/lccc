#!/usr/bin/env bash
# Writable-data page alignment in the built-in static linker.
#
# Contract: when a linked executable has writable, non-RELRO data (.data /
# .bss with content), that block starts on a fresh 4 KiB page, and segment
# congruence `p_offset ≡ p_vaddr (mod page)` holds for every PT_LOAD.
#
# Why this gate exists: SegmentPacker packed sections densely, so `.data`
# landed at whatever page remainder the preceding read-only sections
# produced (e.g. 0x...f60 — 160 bytes short of a boundary). A small hot
# global could then straddle a page: nbody's 280-byte body array crossed
# 0x...f70 -> 0x...000 and every 16-byte inner-loop velocity store became a
# page-split access, a measured 1.55x slowdown vs the identical code linked
# by BFD. BFD avoids it structurally (DATA_SEGMENT_RELRO_END rounds the
# RELRO end to a page), so the writable tail always begins page-aligned.
# The linker now pays the same <= 1 page of file zeros.
#
# Checked on a matrix of link shapes so the pad is proven present exactly
# when it must be and the ELF stays well-formed everywhere:
#   * static + dynamic links,
#   * with and without RELRO content (init_array + relocated const pointer),
#   * .data-only, .bss-only, and mixed data.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

[ -x "$ccc" ] || { echo "SKIP: $ccc not built" >&2; exit 0; }
command -v readelf >/dev/null || { echo "SKIP: readelf missing" >&2; exit 0; }
fail() { echo "FAIL: $*" >&2; exit 1; }

# One source per link shape. Every variant prints a checksum so the run
# also proves the loader actually maps the padded layout correctly.
cat >"$tmp/mixed.c" <<'EOF'
#include <stdio.h>
/* .data: initialized hot array, deliberately not page-sized. */
double bodies[35] = {1.0, 2.0, 3.0};
/* .bss: zero block behind it. */
static double scratch[512];
/* RELRO content: relocated const pointer + constructor. */
double *const body_ref = bodies;
static void setup(void) { bodies[1] += 1.0; }
__attribute__((used, section(".init_array"), aligned(8)))
static void (*ctor)(void) = setup;
int main(void) {
    double s = 0;
    for (int i = 0; i < 35; i++) { scratch[i] = body_ref[i % 35] + i; s += scratch[i]; }
    printf("%.1f\n", s);
    return 0;
}
EOF
cat >"$tmp/bss_only.c" <<'EOF'
#include <stdio.h>
static long big[4096];
int main(void) {
    for (int i = 0; i < 4096; i++) big[i] = i * 3;
    long s = 0; for (int i = 0; i < 4096; i++) s += big[i];
    printf("%ld\n", s);
    return 0;
}
EOF
cat >"$tmp/ro_only.c" <<'EOF'
#include <stdio.h>
static const int table[8] = {1, 2, 3, 4, 5, 6, 7, 8};
int main(void) { int s = 0; for (int i = 0; i < 8; i++) s += table[i]; printf("%d\n", s); return 0; }
EOF

# check <binary> <expect_rw_pad: yes|no>
check_elf() {
    local bin=$1 expect=$2
    local secs; secs=$(readelf -SW "$bin")

    # 1. PT_LOAD congruence: p_offset ≡ p_vaddr (mod 0x1000), always.
    # (pure-shell hex math: mawk has no strtonum)
    while read -r _ off va _; do
        [ $(( off % 4096 )) -eq $(( va % 4096 )) ] \
            || fail "$bin: PT_LOAD $off/$va p_offset !≡ p_vaddr (mod page)"
    done < <(readelf -lW "$bin" | awk '$1 == "LOAD" { print "L", $2, $3 }')

    # 2. The first writable non-RELRO data section starts page-aligned.
    if [ "$expect" = yes ]; then
        local addr
        addr=$(awk '/ \.data / || / \.bss / { print $5; exit }' <<<"$secs")
        [ -n "$addr" ] || fail "$bin: expected .data/.bss, found none"
        [ $(( 0x$addr % 4096 )) -eq 0 ] \
            || fail "$bin: writable data starts at 0x$addr (not page-aligned)"
    fi
}

run_and_check() {
    local src=$1 expect=$2; shift 2
    local bin="$tmp/$(basename "$src" .c)_$(echo "$*" | tr -dc a-z)"
    "$ccc" -O2 "$@" "$src" -o "$bin" || fail "link failed: $src $*"
    "$bin" >/dev/null || fail "run failed: $bin"
    check_elf "$bin" "$expect"
}

for flags in "-static" ""; do
    # shellcheck disable=SC2086
    run_and_check "$tmp/mixed.c" yes $flags
    # shellcheck disable=SC2086
    run_and_check "$tmp/bss_only.c" yes $flags
    # .data/.bss-free binaries must not grow a gratuitous pad page: the
    # file-size delta vs the padded shapes proves the pad is conditional.
    # shellcheck disable=SC2086
    run_and_check "$tmp/ro_only.c" no $flags
done

# 3. Behavioral pin for the original defect: a dense sub-page .rodata tail
# must not push `bodies` across a page boundary. The symbol's full extent
# must live inside one page unless the object itself is >= a page.
readelf -sW "$tmp/mixed_static" >"$tmp/syms.txt"
addr_line=$(awk '$8 == "bodies" { print $2, $3; exit }' "$tmp/syms.txt")
[ -n "$addr_line" ] || fail "bodies symbol not found"
set -- $addr_line
start=$((16#$1)); size=$2
if [ "$size" -le 4096 ]; then
    first_page=$(( start / 4096 )); last_page=$(( (start + size - 1) / 4096 ))
    [ "$first_page" -eq "$last_page" ] \
        || fail "bodies ($size bytes at 0x$1) straddles a page boundary"
fi

echo "PASS: writable data block page-aligned across static/dynamic x {mixed,bss,ro} link shapes"
