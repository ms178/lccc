#!/usr/bin/env bash
# The preprocessor builtins that are not plain replacement lists, and the two
# macros whose value comes from the host clock.
#
# What this pins, and why each part needs a test of its own:
#
#   * `__INCLUDE_LEVEL__` is 0 in the main file and one deeper per `#include`,
#     including the files reached through `-include` (GCC and Clang both report
#     level 1 inside a force-included file: it is as if the `#include` were the
#     first line of the main file, so `-include` is NOT level 0). The value is
#     the *use site's* level, so a header that defines an alias macro and
#     another header that uses it must both see 2.
#
#   * `#undef` of a builtin is permanent for the rest of the translation unit,
#     and `#define` after it takes over the name. `__FILE__`, `__FILE__`-style
#     re-materialisation and the per-token `__LINE__`/`__COUNTER__` used to
#     ignore both rules: a single `#include` resurrected an `#undef`ed
#     `__FILE__`, and `#define __LINE__ 42` was ignored outright.
#
#   * `__TIMESTAMP__` is the modification time of the *current* file, rendered
#     in the host's local zone in `ctime` form (24 characters, space-padded
#     day). With `SOURCE_DATE_EPOCH` set it is that instant in UTC, which is
#     Clang's rule and the one that makes reproducible builds reproducible --
#     GCC leaves `__TIMESTAMP__` alone and would leak the mtime into the object
#     file. A file with no readable mtime gets the documented placeholder rather
#     than a guess (GCC reports whatever `/dev/stdin` happens to be, Clang
#     claims 1970; both are answers about the process, not about the file).
#
#   * `__DATE__` and `__TIME__` are the host's LOCAL wall clock, and UTC only
#     when `SOURCE_DATE_EPOCH` supplies the instant. lccc used to render them in
#     UTC unconditionally, which disagrees with both oracles for every hour
#     outside UTC+0.
#
#   * the level belongs to the *file stack*, not to the line map: `#line 100
#     "other.c"` moves `__FILE__` and must leave `__INCLUDE_LEVEL__` alone, and
#     a `-include` chain nests (1, then 2) exactly like an `#include` one.
#
#   * the same is true where the preprocessor is driven as a filter for another
#     language: `-x assembler-with-cpp` is how the kernel preprocesses `.S` files,
#     and both oracles report level 0 in the `.s` file and level 1 inside the
#     header it includes.
#
#   * `SOURCE_DATE_EPOCH` is validated by GCC's rule -- a non-negative decimal
#     integer no greater than 253402300799 -- and a value that breaks it is
#     diagnosed at the token that asked, with the wall clock as the fallback.
#     lccc used to accept what both oracles reject, and to expand `-1` to the
#     1969 date and `253402300800` to "Jan  1 10000".
#
#   * `-dM` lists feature macros, not the compiler's own builtins: neither gcc-16
#     nor clang-23 prints `__FILE__`, `__LINE__`, `__DATE__`, `__TIME__`,
#     `__TIMESTAMP__`, `__INCLUDE_LEVEL__`, `__BASE_FILE__` or `__COUNTER__` --
#     but a definition the *source* supplied of one of those names is listed,
#     whether it came from `-D` or from a `#define` in an included header.
#
# Every expectation is stated twice where that is possible: once as an exact
# value (so the test cannot drift with lccc) and once as "the oracles agree"
# (so the expectation cannot drift with the author).
set -uo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
CLANG=${CLANG:-clang-23}
tmp=${TMPDIR:-/tmp}/lccc-pp-builtins.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

status=0
fail() { echo "FAIL: $*" >&2; status=1; }
note() { [ -n "${VERBOSE:-}" ] && echo "  $*"; }

have_gcc=0; command -v "$GCC" >/dev/null 2>&1 && have_gcc=1
have_clang=0; command -v "$CLANG" >/dev/null 2>&1 && have_clang=1
if [ "$have_gcc" -eq 0 ] && [ "$have_clang" -eq 0 ]; then
    echo "SKIP: no oracle compiler (gcc-16/clang-23) available" >&2
fi

# Exact assertions read the raw `-E -P` output with grep -F, so a value with
# meaningful internal spaces (`"Nov  2 2026"`) is compared literally.
preprocess() { "$1" -E -P "${@:3}" "$2" 2>/dev/null; }
# Oracle comparisons collapse whitespace runs on BOTH sides instead: the two
# compilers format differently (line breaks, indentation) but produce the same
# token sequence, and token sequence is the contract.
# Leading/trailing whitespace is stripped as well: one side may end with a
# newline the other does not, and that is formatting, not content.
norm() { tr -s ' \t\n' ' ' | sed -e 's/^ //' -e 's/ $//'; }
expect_line() {
    # $1 = haystack function output, $2 = label, $3 = exact line fragment
    grep -qF -- "$3" <<<"$1" || fail "$2: expected '$3' in: $1"
}

# --- 1. __INCLUDE_LEVEL__: nesting, aliases, #if, -include -------------------
cat >"$tmp/main.c" <<'C'
int main_level = __INCLUDE_LEVEL__;
#include "a.h"
int via_alias = LEVEL_ALIAS;
#if __INCLUDE_LEVEL__ == 0
int main_if_ok = 1;
#else
int main_if_ok = 0;
#endif
C
cat >"$tmp/a.h" <<'C'
int a_level = __INCLUDE_LEVEL__;
#define LEVEL_ALIAS __INCLUDE_LEVEL__
#include "b.h"
C
cat >"$tmp/b.h" <<'C'
int b_level = __INCLUDE_LEVEL__;
int b_alias_level = LEVEL_ALIAS;
C
cat >"$tmp/prelude.h" <<'C'
int prelude_level = __INCLUDE_LEVEL__;
C

expect_include_levels() {
    # $1 = label, $2 = preprocessed text
    local label=$1 text=$2
    for want in "int main_level = 0;" "int a_level = 1;" "int b_level = 2;" \
                "int b_alias_level = 2;" "int via_alias = 0;" "int main_if_ok = 1;"; do
        expect_line "$text" "$label" "$want"
    done
}

mine=$(preprocess "$CCC" "$tmp/main.c")
expect_include_levels "include level (lccc)" "$mine"
note "include level: $mine"
if [ "$have_gcc" -eq 1 ]; then
    ref=$(preprocess "$GCC" "$tmp/main.c")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "include level: gcc-16 disagrees: '$ref'"
fi
if [ "$have_clang" -eq 1 ]; then
    ref=$(preprocess "$CLANG" "$tmp/main.c")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "include level: clang-23 disagrees: '$ref'"
fi

# A force-included file runs at level 1 (GCC 16.2 and Clang 23.1.4 agree), not 0.
printf 'int plain_level = __INCLUDE_LEVEL__;\n' >"$tmp/plain.c"
mine=$(preprocess "$CCC" "$tmp/plain.c" -include "$tmp/prelude.h")
expect_line "$mine" "-include" "int prelude_level = 1;"
expect_line "$mine" "-include" "int plain_level = 0;"
if [ "$have_gcc" -eq 1 ]; then
    ref=$(preprocess "$GCC" "$tmp/plain.c" -include "$tmp/prelude.h")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "-include: gcc-16 disagrees: '$ref'"
fi

# --- 2. #undef is permanent, #define afterwards takes the name --------------
cat >"$tmp/undef.h" <<'C'
#undef __INCLUDE_LEVEL__
#undef __TIMESTAMP__
#undef __FILE__
#undef __LINE__
#undef __COUNTER__
C
cat >"$tmp/undef.c" <<'C'
#include "undef.h"
#ifdef __FILE__
int file_defined = 1;
#else
int file_defined = 0;
#endif
#ifdef __LINE__
int line_defined = 1;
#else
int line_defined = 0;
#endif
#ifdef __COUNTER__
int counter_defined = 1;
#else
int counter_defined = 0;
#endif
#ifdef __INCLUDE_LEVEL__
int level_defined = 1;
#else
int level_defined = 0;
#endif
#ifdef __TIMESTAMP__
int timestamp_defined = 1;
#else
int timestamp_defined = 0;
#endif
const char *f = __FILE__;
int l = __LINE__;
int c = __COUNTER__;
int v = __INCLUDE_LEVEL__;
const char *t = __TIMESTAMP__;
C
mine=$(preprocess "$CCC" "$tmp/undef.c")
# The identifiers survive as identifiers, and every #ifdef is false.  Using them
# after this point is a compile error, exactly as with the oracles.
for want in "int file_defined = 0;" "int line_defined = 0;" "int counter_defined = 0;" \
            "int level_defined = 0;" "int timestamp_defined = 0;" \
            "const char *f = __FILE__;" "int l = __LINE__;" "int c = __COUNTER__;" \
            "int v = __INCLUDE_LEVEL__;" "const char *t = __TIMESTAMP__;"; do
    expect_line "$mine" "#undef" "$want"
done
if [ "$have_gcc" -eq 1 ]; then
    ref=$(preprocess "$GCC" "$tmp/undef.c")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "#undef: gcc-16 disagrees: '$ref'"
fi
if [ "$have_clang" -eq 1 ]; then
    # clang warns (as it should) and keeps going; the tokens are what matter.
    ref=$(preprocess "$CLANG" "$tmp/undef.c")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "#undef: clang-23 disagrees: '$ref'"
fi
# ... and the removal is a compile error, not a silently re-materialised macro.
if "$CCC" -c "$tmp/undef.c" -o "$tmp/undef.o" 2>"$tmp/undef.err"; then
    fail "#undef: the identifiers must be undeclared afterwards, but lccc compiled the TU"
fi
grep -q "undeclared" "$tmp/undef.err" || fail "#undef: expected an 'undeclared' diagnostic, got: $(head -3 "$tmp/undef.err")"

cat >"$tmp/redefine.c" <<'C'
#undef __LINE__
#define __LINE__ 42
#undef __TIMESTAMP__
#define __TIMESTAMP__ "from the source"
int l = __LINE__;
const char *t = __TIMESTAMP__;
C
mine=$(preprocess "$CCC" "$tmp/redefine.c")
expect_line "$mine" "#define after #undef" "int l = 42;"
expect_line "$mine" "#define after #undef" 'const char *t = "from the source";'
if [ "$have_gcc" -eq 1 ]; then
    ref=$(preprocess "$GCC" "$tmp/redefine.c")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "#define after #undef: gcc-16 disagrees: '$ref'"
fi

# A definition the *source* supplied wins over the compiler's own: the header
# below must see "mine", not its own name. Both oracles keep the user's macro
# across the include; re-materialising the builtin inside the header is what lccc
# used to do.
printf 'const char *in_header = __FILE__;\n' >"$tmp/inner.h"
cat >"$tmp/userfile.c" <<'C'
#define __FILE__ "mine"
#define __DATE__ "user date"
#include "inner.h"
const char *after = __FILE__;
const char *date_here = __DATE__;
C
mine=$(preprocess "$CCC" "$tmp/userfile.c")
expect_line "$mine" "user __FILE__" 'const char *in_header = "mine";'
expect_line "$mine" "user __FILE__" 'const char *after = "mine";'
expect_line "$mine" "user __DATE__" 'const char *date_here = "user date";'
if [ "$have_gcc" -eq 1 ]; then
    ref=$(preprocess "$GCC" "$tmp/userfile.c")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "user __FILE__: gcc-16 disagrees: '$ref'"
fi
if [ "$have_clang" -eq 1 ]; then
    ref=$(preprocess "$CLANG" "$tmp/userfile.c")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "user __FILE__: clang-23 disagrees: '$ref'"
fi

# --- 3. __TIMESTAMP__: per file, local zone, SOURCE_DATE_EPOCH --------------
# 981173106 = 2001-02-03T04:05:06Z (a Saturday), 946684798 = 1999-12-31T23:59:58Z.
printf 'const char *stamp_a = __TIMESTAMP__;\n' >"$tmp/stamp_a.h"
touch -d "@981173106" "$tmp/stamp_a.h"
printf 'const char *stamp_b = __TIMESTAMP__;\n' >"$tmp/stamp_b.h"
touch -d "@946684798" "$tmp/stamp_b.h"
cat >"$tmp/stamp.c" <<'C'
const char *stamp_main = __TIMESTAMP__;
#include "stamp_a.h"
#include "stamp_b.h"
C
touch -d "@1234567890" "$tmp/stamp.c"

check_stamp() {
    # $1 = TZ ("" for the inherited one), $2 = label, $3.. extra flags
    local tz=$1 label=$2
    shift 2
    local out expect_a expect_b expect_main
    if [ -n "$tz" ]; then
        out=$(TZ="$tz" "$CCC" -E -P "$@" "$tmp/stamp.c" 2>/dev/null)
    else
        out=$("$CCC" -E -P "$@" "$tmp/stamp.c" 2>/dev/null)
    fi
    if [ -n "$tz" ]; then
        expect_a=$(TZ="$tz" date -d "@981173106" +"%a %b %e %H:%M:%S %Y")
        expect_b=$(TZ="$tz" date -d "@946684798" +"%a %b %e %H:%M:%S %Y")
        expect_main=$(TZ="$tz" date -d "@1234567890" +"%a %b %e %H:%M:%S %Y")
    else
        expect_a=$(date -d "@981173106" +"%a %b %e %H:%M:%S %Y")
        expect_b=$(date -d "@946684798" +"%a %b %e %H:%M:%S %Y")
        expect_main=$(date -d "@1234567890" +"%a %b %e %H:%M:%S %Y")
    fi
    expect_line "$out" "$label" "const char *stamp_a = \"$expect_a\";"
    expect_line "$out" "$label" "const char *stamp_b = \"$expect_b\";"
    expect_line "$out" "$label" "const char *stamp_main = \"$expect_main\";"
}

# UTC: the exact strings are stated here, not derived, so the gate is not a
# comparison of the implementation with itself.
out=$(TZ=UTC "$CCC" -E -P "$tmp/stamp.c")
expect_line "$out" "timestamp/UTC" 'const char *stamp_a = "Sat Feb  3 04:05:06 2001";'
expect_line "$out" "timestamp/UTC" 'const char *stamp_b = "Fri Dec 31 23:59:58 1999";'
expect_line "$out" "timestamp/UTC" 'const char *stamp_main = "Fri Feb 13 23:31:30 2009";'
if [ "$have_gcc" -eq 1 ]; then
    ref=$(TZ=UTC "$GCC" -E -P "$tmp/stamp.c" 2>/dev/null | norm)
    mine=$(printf '%s\n' "$out" | norm)
    [ "$mine" = "$ref" ] || fail "timestamp/UTC: gcc-16 disagrees: '$ref'"
fi

# Local zone: a half-hour offset (Asia/Kolkata, no DST) and a DST-observing zone
# whose February offset differs from its July one (America/New_York, UTC-5 in
# winter). Both are cross-checked against coreutils' zone conversion, an oracle
# independent of every C compiler here.
for tz in Asia/Kolkata America/New_York; do
    check_stamp "$tz" "timestamp/TZ=$tz"
    if [ "$have_gcc" -eq 1 ]; then
        mine=$(TZ="$tz" "$CCC" -E -P "$tmp/stamp.c" 2>/dev/null | norm)
        ref=$(TZ="$tz" "$GCC" -E -P "$tmp/stamp.c" 2>/dev/null | norm)
        [ "$mine" = "$ref" ] || fail "timestamp/TZ=$tz: gcc-16 disagrees: '$ref'"
    fi
done

# SOURCE_DATE_EPOCH wins over the file's mtime (Clang's rule) and is rendered in
# UTC, so the same source reproduces byte for byte in any zone.
out=$(TZ=Asia/Kolkata SOURCE_DATE_EPOCH=1000000000 "$CCC" -E -P "$tmp/stamp.c")
expect_line "$out" "timestamp/SOURCE_DATE_EPOCH" 'const char *stamp_a = "Sun Sep  9 01:46:40 2001";'
expect_line "$out" "timestamp/SOURCE_DATE_EPOCH" 'const char *stamp_main = "Sun Sep  9 01:46:40 2001";'
if [ "$have_clang" -eq 1 ]; then
    mine=$(printf '%s\n' "$out" | norm)
    ref=$(TZ=Asia/Kolkata SOURCE_DATE_EPOCH=1000000000 "$CLANG" -E -P "$tmp/stamp.c" 2>/dev/null | norm)
    [ "$mine" = "$ref" ] || fail "timestamp/SOURCE_DATE_EPOCH: clang-23 disagrees: '$ref'"
fi

# No readable mtime: the documented placeholder, not a guess about the process.
out=$(printf 'const char *stamp = __TIMESTAMP__;\n' | "$CCC" -E -P -x c - 2>/dev/null)
expect_line "$out" "timestamp/stdin" 'const char *stamp = "??? ??? ?? ??:??:?? ????";'

# --- 4. __DATE__ and __TIME__ follow the host's zone ------------------------
printf 'const char *d = __DATE__;\nconst char *t = __TIME__;\n' >"$tmp/dt.c"
for tz in UTC Asia/Kolkata America/New_York Pacific/Chatham; do
    # The wall clock can cross a second boundary between the two runs; when it
    # does, the observation is void rather than a failure, so re-sample.
    mine=''; ref=''; settled=0
    for _ in 1 2 3 4 5; do
        before=$(date +%s)
        mine=$(TZ="$tz" "$CCC" -E -P "$tmp/dt.c" 2>/dev/null | norm)
        if [ "$have_gcc" -eq 0 ]; then settled=1; break; fi
        ref=$(TZ="$tz" "$GCC" -E -P "$tmp/dt.c" 2>/dev/null | norm)
        after=$(date +%s)
        [ "$before" = "$after" ] && { settled=1; break; }
    done
    [ "$settled" -eq 1 ] || note "date/time/TZ=$tz: sampled across a second boundary"
    # The date and time must be the zone's, not UTC: compare against the host
    # clock rendered in that zone by coreutils.
    # `norm` collapsed the padding inside the quoted date, so the expectation is
    # normalised the same way: both sides say what they say, whitespace aside.
    want_date=$(TZ="$tz" date +"%b %e %Y" | norm)
    want_time=$(TZ="$tz" date +%H:%M)
    expect_line "$mine" "date/TZ=$tz" "d = \"$want_date\""
    expect_line "$mine" "time/TZ=$tz" "t = \"$want_time"
    if [ -n "$ref" ]; then
        [ "$mine" = "$ref" ] || fail "date/time/TZ=$tz: gcc-16 disagrees: '$ref'"
    fi
done
# SOURCE_DATE_EPOCH makes both reproducible in UTC regardless of zone.
out=$(TZ=America/New_York SOURCE_DATE_EPOCH=1000000000 "$CCC" -E -P "$tmp/dt.c")
expect_line "$out" "date/SOURCE_DATE_EPOCH" 'const char *d = "Sep  9 2001";'
expect_line "$out" "time/SOURCE_DATE_EPOCH" 'const char *t = "01:46:40";'

# --- 5. The level belongs to the file stack, not to the line map ------------
# `#line` rewrites the presumed file and line; it does not move the token to
# another file, so the level must not move either (measured on both oracles).
cat >"$tmp/line.c" <<'C'
#line 100 "other.c"
int after_line = __INCLUDE_LEVEL__;
const char *presumed = __FILE__;
C
mine=$(preprocess "$CCC" "$tmp/line.c")
expect_line "$mine" "#line level" "int after_line = 0;"
expect_line "$mine" "#line file" 'const char *presumed = "other.c";'
for oracle in "$GCC:$have_gcc" "$CLANG:$have_clang"; do
    oracle_bin=${oracle%:*}
    [ "${oracle##*:}" = "1" ] || continue
    ref=$(preprocess "$oracle_bin" "$tmp/line.c")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "#line: $oracle_bin disagrees: '$ref'"
done

# --- 6. A -include chain nests like an #include chain -----------------------
# A force-included file runs at level 1; the file it includes runs at level 2.
cat >"$tmp/deep_driver.c" <<'C'
int driver_level = __INCLUDE_LEVEL__;
C
cat >"$tmp/deep_outer.h" <<'C'
#include "deep_inner.h"
int outer_level = __INCLUDE_LEVEL__;
C
cat >"$tmp/deep_inner.h" <<'C'
int inner_level = __INCLUDE_LEVEL__;
C
mine=$(preprocess "$CCC" "$tmp/deep_driver.c" -I"$tmp" -include deep_outer.h)
expect_line "$mine" "-include chain" "int inner_level = 2;"
expect_line "$mine" "-include chain" "int outer_level = 1;"
expect_line "$mine" "-include chain" "int driver_level = 0;"
for oracle in "$GCC:$have_gcc" "$CLANG:$have_clang"; do
    oracle_bin=${oracle%:*}
    [ "${oracle##*:}" = "1" ] || continue
    ref=$(preprocess "$oracle_bin" "$tmp/deep_driver.c" -I"$tmp" -include deep_outer.h)
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "-include chain: $oracle_bin disagrees: '$ref'"
done

# --- 7. The level in assembler-with-cpp -------------------------------------
# `-x assembler-with-cpp` is how the kernel preprocesses `.S` files: the builtin
# has to work in that mode too, and the header it includes is one level deeper.
cat >"$tmp/asm_level.s" <<'C'
#include "asm_level.h"
.data
.byte __INCLUDE_LEVEL__
C
printf 'int in_header = __INCLUDE_LEVEL__;\n' >"$tmp/asm_level.h"
mine=$(preprocess "$CCC" "$tmp/asm_level.s" -x assembler-with-cpp -I"$tmp")
expect_line "$mine" "asm-with-cpp" "int in_header = 1;"
expect_line "$mine" "asm-with-cpp" ".byte 0"
for oracle in "$GCC:$have_gcc" "$CLANG:$have_clang"; do
    oracle_bin=${oracle%:*}
    [ "${oracle##*:}" = "1" ] || continue
    ref=$(preprocess "$oracle_bin" "$tmp/asm_level.s" -x assembler-with-cpp -I"$tmp")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "asm-with-cpp: $oracle_bin disagrees: '$ref'"
done

# --- 8. A header that takes over a name keeps it, and -dM says so -----------
# The definition arrives through an `#include`, not through `-D`, so the
# ownership flag has to be set by the ordinary `#define` path.
cat >"$tmp/takeover_header.h" <<'C'
#define __TIME__ "from header"
#define __FILE__ "header file"
C
cat >"$tmp/takeover_main.c" <<'C'
#include "takeover_header.h"
const char *t = __TIME__;
const char *f = __FILE__;
C
mine=$(preprocess "$CCC" "$tmp/takeover_main.c" -I"$tmp")
expect_line "$mine" "in-source takeover" 'const char *t = "from header";'
expect_line "$mine" "in-source takeover" 'const char *f = "header file";'
mine_dump=$("$CCC" -dM -E -I"$tmp" "$tmp/takeover_main.c")
for spec in '__TIME__ "from header"' '__FILE__ "header file"'; do
    grep -qF -- "#define $spec" <<<"$mine_dump" \
        || fail "-dM: an in-source definition must be listed: '#define $spec'"
done
for oracle in "$GCC:$have_gcc" "$CLANG:$have_clang"; do
    oracle_bin=${oracle%:*}
    [ "${oracle##*:}" = "1" ] || continue
    ref=$(preprocess "$oracle_bin" "$tmp/takeover_main.c" -I"$tmp")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "in-source takeover: $oracle_bin disagrees: '$ref'"
    ref_dump=$("$oracle_bin" -dM -E -I"$tmp" "$tmp/takeover_main.c" 2>/dev/null)
    for name in __TIME__ __FILE__; do
        mine_listed=$(grep -c "^#define $name " <<<"$mine_dump")
        ref_listed=$(grep -c "^#define $name " <<<"$ref_dump")
        [ "$mine_listed" = "$ref_listed" ] \
            || fail "-dM: in-source '$name' listing ($mine_listed) differs from $oracle_bin ($ref_listed)"
    done
done

# --- 9a. SOURCE_DATE_EPOCH is validated the way GCC validates it -----------
# Both reference compilers reject a value that is not a non-negative decimal
# integer no greater than 253402300799 (the last second whose `__DATE__` still
# fits a four-digit year), and both then fall back to the wall clock. lccc used to
# accept what they reject -- `-1` expanded to the 1969 date, `253402300800` to
# "Jan  1 10000", instants no reference compiler produces. It now diagnoses them
# and keeps GCC's acceptance rule (`+` and leading whitespace allowed, trailing
# garbage not), which is the permissive one of the two oracles: Clang also rejects
# the sign and the whitespace, so following GCC cannot fail a build that worked
# before.
printf 'const char *d = __DATE__;\nconst char *t = __TIME__;\n' >"$tmp/sde.c"
printf 'int x;\n' >"$tmp/sde_unused.c"
printf 'const char *ts = __TIMESTAMP__;\n' >"$tmp/sde_stamp.c"

for value in 0 1700000000 +1700000000 01700000000 253402300799; do
    if ! out=$(TZ=UTC SOURCE_DATE_EPOCH="$value" "$CCC" -E -P "$tmp/sde.c" 2>"$tmp/sde.err"); then
        fail "SOURCE_DATE_EPOCH='$value' must be accepted: $(cat "$tmp/sde.err")"
        continue
    fi
    [ -s "$tmp/sde.err" ] && fail "SOURCE_DATE_EPOCH='$value' must be silent: $(cat "$tmp/sde.err")"
    if [ "$have_gcc" -eq 1 ]; then
        ref=$(TZ=UTC SOURCE_DATE_EPOCH="$value" "$GCC" -E -P "$tmp/sde.c" 2>/dev/null)
        [ "$(printf '%s' "$out" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
            || fail "SOURCE_DATE_EPOCH='$value': gcc-16 disagrees: '$ref'"
    fi
done

for value in -1 abc 1700000000x 253402300800 99999999999999999999; do
    if TZ=UTC SOURCE_DATE_EPOCH="$value" "$CCC" -E -P "$tmp/sde.c" \
            >"$tmp/sde.out" 2>"$tmp/sde.err"; then
        fail "SOURCE_DATE_EPOCH='$value' must be rejected, but the compile succeeded"
        continue
    fi
    grep -q "SOURCE_DATE_EPOCH" "$tmp/sde.err" \
        || fail "SOURCE_DATE_EPOCH='$value': the diagnostic must name the variable: $(cat "$tmp/sde.err")"
    # The macro still expands, to the sampled value, exactly as GCC's does.
    mine=''; ref=''; settled=0
    for _ in 1 2 3 4 5; do
        before=$(date +%s)
        mine=$(TZ=UTC SOURCE_DATE_EPOCH="$value" "$CCC" -E -P "$tmp/sde.c" 2>/dev/null | norm)
        if [ "$have_gcc" -eq 0 ]; then settled=1; break; fi
        ref=$(TZ=UTC SOURCE_DATE_EPOCH="$value" "$GCC" -E -P "$tmp/sde.c" 2>/dev/null | norm)
        after=$(date +%s)
        [ "$before" = "$after" ] && { settled=1; break; }
    done
    [ "$settled" -eq 1 ] || note "SOURCE_DATE_EPOCH='$value': sampled across a second boundary"
    if [ "$have_gcc" -eq 1 ]; then
        [ "$mine" = "$ref" ] \
            || fail "SOURCE_DATE_EPOCH='$value': the fallback value must match gcc-16: mine='$mine' ref='$ref'"
    fi
done

# An empty value is set-but-invalid, which is not the same as unset.
if TZ=UTC SOURCE_DATE_EPOCH="" "$CCC" -E -P "$tmp/sde.c" >/dev/null 2>"$tmp/sde.err"; then
    fail "SOURCE_DATE_EPOCH='' must be rejected (Clang and gcc both reject it)"
fi

# ... but a malformed value that no macro reads is not an error in GCC, and must
# not be one here: Clang fails such a build, GCC does not, and failing a build
# that used to work is the worse of the two failures.
if ! TZ=UTC SOURCE_DATE_EPOCH=abc "$CCC" -E -P "$tmp/sde_unused.c" >/dev/null 2>"$tmp/sde.err"; then
    fail "an unread malformed SOURCE_DATE_EPOCH must not fail the build: $(cat "$tmp/sde.err")"
fi

# __TIMESTAMP__ consults the same value and diagnoses it the same way.
if TZ=UTC SOURCE_DATE_EPOCH=abc "$CCC" -E -P "$tmp/sde_stamp.c" >/dev/null 2>"$tmp/sde.err"; then
    fail "__TIMESTAMP__ with a malformed SOURCE_DATE_EPOCH must be diagnosed"
fi

# --- 9. -dM excludes the builtins, and includes the feature macros ----------
dump=$("$CCC" -dM -E -x c /dev/null)
for name in __FILE__ __LINE__ __DATE__ __TIME__ __TIMESTAMP__ __INCLUDE_LEVEL__ \
            __BASE_FILE__ __COUNTER__; do
    if grep -q "^#define $name\b" <<<"$dump"; then
        fail "-dM: '$name' must not be listed as a definition"
    fi
done
for name in __GNUC__ __STDC_VERSION__ __x86_64__; do
    if ! grep -q "^#define $name\b" <<<"$dump"; then
        fail "-dM: '$name' must still be listed (the filter must not over-reach)"
    fi
done
# ... but a definition the source supplied IS listed, exactly as both oracles
# list `-D__DATE__="x"`. A name-based filter would hide it. All eight names are
# covered, because each one reaches the table by a different route: four are
# materialised, `__LINE__`/`__COUNTER__` are computed per token, and
# `__BASE_FILE__` is installed once at startup.
user_defs=''
specs=''
for name in __FILE__ __LINE__ __COUNTER__ __DATE__ __TIME__ __TIMESTAMP__ \
            __INCLUDE_LEVEL__ __BASE_FILE__; do
    user_defs="$user_defs -D$name=\"x\""
    specs="$specs $name"
done
# shellcheck disable=SC2086
user_dump=$("$CCC" -dM -E $user_defs -x c /dev/null)
for name in $specs; do
    if ! grep -qF -- "#define $name \"x\"" <<<"$user_dump"; then
        fail "-dM: a user-supplied definition must be listed: '#define $name \"x\"'"
    fi
done
# And exactly once: a second definition of the same name would mean the
# compiler's value was re-materialised over the source's.
for name in $specs; do
    listed=$(printf '%s\n' "$user_dump" | grep -c "^#define $name ")
    [ "$listed" = "1" ] || fail "-dM: '$name' is listed $listed time(s), want 1"
done
for oracle in "$GCC:$have_gcc" "$CLANG:$have_clang"; do
    oracle_bin=${oracle%:*}
    [ "${oracle##*:}" = "1" ] || continue
    # shellcheck disable=SC2086
    ref_dump=$("$oracle_bin" -dM -E $user_defs -x c /dev/null 2>/dev/null)
    for name in $specs; do
        mine_listed=$(printf '%s\n' "$user_dump" | grep -c "^#define $name ")
        ref_listed=$(printf '%s\n' "$ref_dump" | grep -c "^#define $name ")
        [ "$mine_listed" = "$ref_listed" ] \
            || fail "-dM: '$name' listing ($mine_listed) differs from $oracle_bin ($ref_listed)"
    done
done

# --- 10. Degenerate and shadowing definitions of the builtins --------------
# Three shapes that a name-first implementation gets wrong and a table-first one
# gets right for free. Each is cross-checked against both oracles.

# (a) `__TIMESTAMP__` is the *physical* file's mtime even after `#line` rewrites
# the presumed name: the macro asks about a file, and `#line` only lies about
# which one the diagnostics should name.
printf '#line 1 "virt.c"\nconst char *ts = __TIMESTAMP__;\n' >"$tmp/stamp_virt.c"
touch -d '2029-03-04 05:06:07 UTC' "$tmp/stamp_virt.c"
mine=$(TZ=UTC preprocess "$CCC" "$tmp/stamp_virt.c")
expect_line "$mine" "#line timestamp" 'const char *ts = "Sun Mar  4 05:06:07 2029";'
for oracle in "$GCC:$have_gcc" "$CLANG:$have_clang"; do
    oracle_bin=${oracle%:*}
    [ "${oracle##*:}" = "1" ] || continue
    ref=$(TZ=UTC preprocess "$oracle_bin" "$tmp/stamp_virt.c")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "#line timestamp: $oracle_bin disagrees: '$ref'"
done

# (b) An empty user definition wins, so the builtin expands to nothing -- and the
# surrounding tokens are not swallowed with it.
printf '#define __INCLUDE_LEVEL__\nint empty = __INCLUDE_LEVEL__ ;\n' >"$tmp/empty_def.c"
mine=$(preprocess "$CCC" "$tmp/empty_def.c")
[ "$(printf '%s' "$mine" | norm)" = "int empty = ;" ] \
    || fail "empty definition: expected 'int empty = ;', got '$mine'"
for oracle in "$GCC:$have_gcc" "$CLANG:$have_clang"; do
    oracle_bin=${oracle%:*}
    [ "${oracle##*:}" = "1" ] || continue
    ref=$(preprocess "$oracle_bin" "$tmp/empty_def.c")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "empty definition: $oracle_bin disagrees: '$ref'"
done

# (c) A *function-like* user definition leaves a bare mention alone (it is not an
# invocation) and expands when called: both oracles do exactly that, which is why
# `builtin_expands` consults the table instead of the name.
printf '#define __LINE__(x) (x)\nint bare = __LINE__;\nint call_ = __LINE__(7);\n' >"$tmp/fnlike.c"
mine=$(preprocess "$CCC" "$tmp/fnlike.c")
expect_line "$mine" "function-like shadow" "int bare = __LINE__;"
expect_line "$mine" "function-like shadow" "int call_ = (7);"
for oracle in "$GCC:$have_gcc" "$CLANG:$have_clang"; do
    oracle_bin=${oracle%:*}
    [ "${oracle##*:}" = "1" ] || continue
    ref=$(preprocess "$oracle_bin" "$tmp/fnlike.c")
    [ "$(printf '%s' "$mine" | norm)" = "$(printf '%s' "$ref" | norm)" ] \
        || fail "function-like shadow: $oracle_bin disagrees: '$ref'"
done

if [ "$have_gcc" -eq 1 ]; then
    ref=$("$GCC" -dM -E -x c /dev/null)
    for name in __FILE__ __LINE__ __DATE__ __TIME__ __TIMESTAMP__ __INCLUDE_LEVEL__ \
                __BASE_FILE__ __COUNTER__; do
        if grep -q "^#define $name\b" <<<"$ref"; then
            fail "-dM: the oracle lists '$name', so the rule this gate encodes is wrong"
        fi
    done
fi

# --- 11. __BIGGEST_ALIGNMENT__ follows the ISA and IS the _Alignof cap ------
# GCC defines `__BIGGEST_ALIGNMENT__` as its `BIGGEST_ALIGNMENT` macro -- the
# *same* number `min_align_of_type` caps C11 `_Alignof` at -- so it is a
# property of the translation unit's ISA, not a constant: 16 on the x86-64
# baseline, 32 once `-mavx` is enabled, 64 under `-mavx512f`, and the same
# numbers on the `-m32` rows (measured, GCC 16.2). lccc carried a literal 16 in
# two predefined-macro tables while `_Alignof(v32)` in the very same
# translation unit already answered 32 under `-mavx`: the macro reported a
# biggest alignment smaller than a type the compiler itself accepts, so every
# `char buf[__BIGGEST_ALIGNMENT__]` that AVX/AVX-512 code reinterprets as an
# `__m256`/`__m512` was under-aligned. Clang 23.1.4 pins 16 and is
# self-inconsistent in exactly that way (`_Alignof(v64)` is 64 there), so GCC
# is the only coherent oracle and therefore the expectation.
#
# Each row is pinned three ways: the exact expected value (so the gate cannot
# drift with lccc), agreement with gcc-16 when it is present (so the
# expectation cannot drift with the author), and the internal invariant
# `_Alignof(vN) == min(N, __BIGGEST_ALIGNMENT__)` compiled into the same TU (so
# the macro and the cap can never diverge again, oracle or not).
alg_rows=(
    ":16"
    "-march=x86-64:16"
    "-march=x86-64-v2:16"
    "-march=x86-64-v3:32"
    "-march=x86-64-v4:64"
    "-march=znver4:64"
    "-march=nehalem:16"
    "-msse4.2:16"
    "-mavx:32"
    "-mavx2:32"
    "-mavx512f:64"
    "-mno-sse2:16"
    # The ISA request decides, not a denial of the narrower vector state: both
    # oracles keep 32/64 under these (measured), and so must lccc.
    "-mno-sse2 -mavx2:32"
    "-mno-sse -mavx:32"
    # Last flag wins, and an AVX-512 request implies AVX.
    "-mavx512f -mno-avx512f:16"
    "-mno-avx -mavx512f:64"
    "-m32:16"
    "-m32 -mavx2:32"
    "-m32 -mavx512f:64"
)
cat >"$tmp/biggest.c" <<'C'
typedef float v16 __attribute__((vector_size(16)));
typedef float v32 __attribute__((vector_size(32)));
typedef float v64 __attribute__((vector_size(64)));
_Static_assert(__BIGGEST_ALIGNMENT__ >= 16, "the baseline is 16");
_Static_assert(_Alignof(v16) == 16, "v16");
_Static_assert(_Alignof(v32) == (32 < __BIGGEST_ALIGNMENT__ ? 32 : __BIGGEST_ALIGNMENT__),
               "the cap and _Alignof(v32) disagree");
_Static_assert(_Alignof(v64) == (64 < __BIGGEST_ALIGNMENT__ ? 64 : __BIGGEST_ALIGNMENT__),
               "the cap and _Alignof(v64) disagree");
/* an integer constant expression in every context a build uses it in */
char big_buf[__BIGGEST_ALIGNMENT__];
_Static_assert(sizeof big_buf == __BIGGEST_ALIGNMENT__, "not an ICE");
#if __BIGGEST_ALIGNMENT__ >= 32
int wide_ok = 1;
#else
int wide_ok = 0;
#endif
C
for row in "${alg_rows[@]}"; do
    flags=${row%:*}; want=${row##*:}
    # shellcheck disable=SC2086  # the flag list is a word list on purpose
    got=$("$CCC" $flags -dM -E -x c /dev/null 2>/dev/null \
        | awk '/^#define __BIGGEST_ALIGNMENT__ /{print $3}')
    [ "$got" = "$want" ] \
        || fail "__BIGGEST_ALIGNMENT__ [$flags]: expected $want, got '${got:-<undefined>}'"
    if [ "$have_gcc" -eq 1 ]; then
        # shellcheck disable=SC2086
        ref=$("$GCC" $flags -dM -E -x c /dev/null 2>/dev/null \
            | awk '/^#define __BIGGEST_ALIGNMENT__ /{print $3}')
        # An empty ref means gcc-16 rejected the row in this environment (no
        # such row today, but a multilib-less gcc cannot answer -m32 probes);
        # the exact expectation above already pins the value.
        if [ -n "$ref" ] && [ "$ref" != "$got" ]; then
            fail "__BIGGEST_ALIGNMENT__ [$flags]: gcc-16 says $ref, lccc says '${got:-<undefined>}'"
        fi
    fi
    # shellcheck disable=SC2086
    if ! "$CCC" $flags -c -o /dev/null "$tmp/biggest.c" 2>"$tmp/biggest.err"; then
        fail "__BIGGEST_ALIGNMENT__ [$flags]: the macro is not the _Alignof cap of the same TU: $(cat "$tmp/biggest.err")"
    fi
done

[ "$status" -eq 0 ] && echo "preprocessor-builtin-scope: ok"
exit "$status"
