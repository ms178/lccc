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
#   * `-dM` lists feature macros, not the compiler's own builtins: neither gcc-16
#     nor clang-23 prints `__FILE__`, `__LINE__`, `__DATE__`, `__TIME__`,
#     `__TIMESTAMP__`, `__INCLUDE_LEVEL__`, `__BASE_FILE__` or `__COUNTER__`.
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
    printf '%s\n' "$1" | grep -qF "$3" || fail "$2: expected '$3' in: $1"
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

# --- 5. -dM excludes the builtins, and includes the feature macros ----------
dump=$("$CCC" -dM -E -x c /dev/null)
for name in __FILE__ __LINE__ __DATE__ __TIME__ __TIMESTAMP__ __INCLUDE_LEVEL__ \
            __BASE_FILE__ __COUNTER__; do
    if printf '%s\n' "$dump" | grep -q "^#define $name\b"; then
        fail "-dM: '$name' must not be listed as a definition"
    fi
done
for name in __GNUC__ __STDC_VERSION__ __x86_64__; do
    if ! printf '%s\n' "$dump" | grep -q "^#define $name\b"; then
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
    if ! printf '%s\n' "$user_dump" | grep -qF "#define $name \"x\""; then
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

if [ "$have_gcc" -eq 1 ]; then
    ref=$("$GCC" -dM -E -x c /dev/null)
    for name in __FILE__ __LINE__ __DATE__ __TIME__ __TIMESTAMP__ __INCLUDE_LEVEL__ \
                __BASE_FILE__ __COUNTER__; do
        if printf '%s\n' "$ref" | grep -q "^#define $name\b"; then
            fail "-dM: the oracle lists '$name', so the rule this gate encodes is wrong"
        fi
    done
fi

[ "$status" -eq 0 ] && echo "preprocessor-builtin-scope: ok"
exit "$status"
