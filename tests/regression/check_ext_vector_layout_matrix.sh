#!/usr/bin/env bash
# ext_vector_type layout + lane-operation matrix against the Clang oracle.
#
# `check_ext_vector_size.sh` pins a handful of hand-picked widths, and its i686
# leg only runs where the target's glibc development headers are installed.
# Layout is a front-end property, not a libc one, so the i686 half of the
# contract went unverified in every environment that lacked multilib headers
# (minimal sandboxes, containers, and this workspace before the bookworm i686
# sysroot was assembled).  This gate closes that gap with a freestanding
# fixture: 12 element types x 10 lane counts on both targets, at -O0 and -O2,
# each compared byte-for-byte with Clang 23.1.3 for the same target.
#
# The fixture prints `sizeof`/`_Alignof` per shape and one hash over lane
# stores+loads, and carries its own `_Static_assert` layout laws, so a
# disagreement is reported as a size/alignment mismatch, a lane-addressing
# mismatch, or a compile-time law violation rather than as a number in a log.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
gcc_bin=${GCC:-gcc}
# shellcheck source=tests/regression/lib/clang_oracle.sh
source "$root/tests/regression/lib/clang_oracle.sh"
# shellcheck source=tests/regression/i386_exec.sh
source "$root/tests/regression/i386_exec.sh"

require_clang_oracle check_ext_vector_layout_matrix || exit 1
clang=$CLANG_ORACLE_BIN
[ -x "$ccc" ] || { echo "FAIL: compiler not built: $ccc" >&2; exit 1; }
src=$root/tests/regression/ext_vector_layout/layout_matrix.c
[ -f "$src" ] || { echo "FAIL: fixture missing: $src" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# build_and_run <compiler> <bits> <opt> <tag>: compile the fixture for one
# target/optimisation point and capture its output.  A build or run failure is
# a gate failure, never a skip: both compilers accept this fixture by
# construction, so a failure here is a real defect.
#
# The object is compiled by the compiler under test and linked by the HOST
# driver, exactly like the ext-vector ABI grid: this gate measures the layout
# the front end emitted, and a differential must not also vary the link
# environment (a -m32 link is a property of the host toolchain, not of the
# layout answer).
build_and_run() {
    local compiler=$1 bits=$2 opt=$3 tag=$4
    local obj=$tmp/$tag.o exe=$tmp/$tag
    if ! "$compiler" -m"$bits" -O"$opt" -c "$src" -o "$obj" >"$tmp/$tag.log" 2>&1; then
        echo "FAIL: $tag: compile failed" >&2
        sed -n '1,20p' "$tmp/$tag.log" >&2
        return 1
    fi
    if ! "$gcc_bin" -m"$bits" -no-pie "$obj" -o "$exe" >>"$tmp/$tag.log" 2>&1; then
        echo "FAIL: $tag: link failed" >&2
        sed -n '1,20p' "$tmp/$tag.log" >&2
        return 1
    fi
    if ! "$exe" >"$tmp/$tag.out" 2>"$tmp/$tag.err"; then
        echo "FAIL: $tag: run failed" >&2
        sed -n '1,20p' "$tmp/$tag.err" >&2
        return 1
    fi
}

# compare_leg <bits> <opt>: lccc must equal the Clang oracle for this target,
# and lccc's own -O0 and -O2 answers must agree (layout is
# optimisation-invariant; a difference means a lowering path, not a layout
# rule, decided the storage size).
compare_leg() {
    local bits=$1 opt=$2
    build_and_run "$ccc" "$bits" "$opt" "lccc-$bits-$opt" || return 1
    build_and_run "$clang" "$bits" "$opt" "clang-$bits-$opt" || return 1
    if ! diff -u "$tmp/clang-$bits-$opt.out" "$tmp/lccc-$bits-$opt.out" \
        >"$tmp/diff-$bits-$opt.txt"; then
        echo "FAIL: -m$bits -O$opt: layout/lane mismatch against Clang $CLANG_ORACLE_VERSION" >&2
        sed -n '1,40p' "$tmp/diff-$bits-$opt.txt" >&2
        return 1
    fi
    printf 'PASS: ext_vector layout matrix -m%s -O%s\n' "$bits" "$opt"
}

ran_m32=0
for bits in 64 32; do
    if [[ $bits == 32 ]] && ! i386_run_or_skip "$gcc_bin"; then
        if [[ ${LCCC_REQUIRE_I686:-0} == 1 ]]; then exit 1; fi
        continue
    fi
    [[ $bits == 32 ]] && ran_m32=1
    for opt in 0 2; do
        compare_leg "$bits" "$opt" || exit 1
    done
done

# Optimisation invariance is checked where both points ran for the same target.
for bits in 64 32; do
    [[ $bits == 32 && $ran_m32 == 0 ]] && continue
    if ! diff -u "$tmp/lccc-$bits-0.out" "$tmp/lccc-$bits-2.out" \
        >"$tmp/inv-$bits.txt"; then
        echo "FAIL: -m$bits: lccc layout differs between -O0 and -O2" >&2
        sed -n '1,40p' "$tmp/inv-$bits.txt" >&2
        exit 1
    fi
    printf 'PASS: ext_vector layout matrix -m%s is -O0/-O2 invariant\n' "$bits"
done

# ── controls ───────────────────────────────────────────────────────────────
# 1. The comparison must be live: perturb one emitted byte count and require
#    the difference detector to notice it.  A gate whose comparison step cannot
#    fail would pass against any compiler.
if ! cmp -s "$tmp/clang-64-2.out" "$tmp/lccc-64-2.out"; then
    echo "FAIL: control: the -m64 -O2 leg did not compare equal" >&2
    exit 1
fi
# The fixture's first row is `b1 1 1 1` (one lane of `_Bool`).  Naming a
# different size for it must be detected; using the row's exact text keeps the
# mutation deterministic and dependency-free.
sed '1s/^b1 1 1 1$/b1 1 9 1/' "$tmp/lccc-64-2.out" >"$tmp/perturbed.out"
if cmp -s "$tmp/lccc-64-2.out" "$tmp/perturbed.out"; then
    echo "FAIL: control: the fixture's first row is not 'b1 1 1 1'; the size control cannot run" >&2
    exit 1
fi
if cmp -s "$tmp/clang-64-2.out" "$tmp/perturbed.out"; then
    echo "FAIL: control: a perturbed size row still compares equal" >&2
    exit 1
fi
printf 'PASS: control: layout comparison detects a perturbed size row\n'

# 2. `sizeof` must be evaluated in the *target's* width domain: three
#    `unsigned long` lanes are 32 bytes on LP64 and 16 on i686, so a
#    `_Static_assert` naming the LP64 answer must pass for -m64 and be rejected
#    for -m32.  An i686 build that accepted it would be answering with the
#    host's integer model -- the exact mistake P0.1 was filed about.
cat >"$tmp/law_probe.c" <<'PCLAW'
typedef unsigned long u3 __attribute__((ext_vector_type(3)));
_Static_assert(sizeof(u3) == 32, "the LP64 three-lane unsigned long layout");
int main(void) { return 0; }
PCLAW
# Compile-only (-c): the control is about the assert, so a link failure in a
# host without a 32-bit crt must not be able to stand in for a rejection.
if ! "$ccc" -m64 -c "$tmp/law_probe.c" -o "$tmp/law64.o" >"$tmp/law64.log" 2>&1; then
    echo "FAIL: control: the LP64 three-lane layout did not hold on -m64" >&2
    sed -n '1,10p' "$tmp/law64.log" >&2
    exit 1
fi
if "$ccc" -m32 -c "$tmp/law_probe.c" -o "$tmp/law32.o" >"$tmp/law32.log" 2>&1; then
    echo "FAIL: control: -m32 accepted the LP64 three-lane width" >&2
    exit 1
fi
if ! grep -q 'the LP64 three-lane unsigned long layout' "$tmp/law32.log"; then
    echo "FAIL: control: -m32 rejected the LP64 assert for the wrong reason" >&2
    sed -n '1,10p' "$tmp/law32.log" >&2
    exit 1
fi
printf 'PASS: control: sizeof is evaluated in the target width domain (LP64 accepts, i686 rejects)\n'

# The fixture is only meaningful if it really carries the layout laws and the
# lane probe; a truncated or emptied fixture would pass every leg vacuously.
# Every shape carries one lane-storage law and one alignment law; the laws
# themselves are `_Static_assert`s inside the three macros above, so counting
# invocations AND definitions makes an emptied macro body fail too.
laws=$(grep -cE '^(BOOL_PACKED|LANES_AT_LEAST|ALIGN_LAW)\(' "$src" || true)
asserts=$(grep -c '_Static_assert' "$src" || true)
if (( laws < 240 || asserts < 6 )); then
    echo "FAIL: fixture carries only $laws layout laws ($asserts assertions); expected the full matrix" >&2
    exit 1
fi
if ! grep -q '^lanes ' "$tmp/lccc-64-0.out"; then
    echo "FAIL: fixture emitted no lane-operation hash" >&2
    exit 1
fi
printf 'PASS: ext_vector layout matrix (%d shapes, %d laws)\n' \
    "$(grep -c '^typedef' "$src")" "$laws"
