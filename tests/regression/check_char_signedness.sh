#!/usr/bin/env bash
# Plain-`char` signedness must be ONE answer, not three.
#
# A `char` that compares as unsigned while `<limits.h>` still says
# `CHAR_MIN == -128` is a miscompile factory: code that tests
# `c < 0`, tables indexed by `(unsigned char)c`, and anything that checks
# `#ifdef __CHAR_UNSIGNED__` all disagree with each other.  The compiler has
# three places that must agree, and they are derived from one state
# (`common::types::char_is_unsigned()`):
#
#   1. the *type system* — `(char)-1 < 0` must reflect the target/flag;
#   2. the *preprocessor* — `__CHAR_UNSIGNED__` is defined iff that state
#      says unsigned (it used to be defined only inside the AArch64 target
#      block, so RISC-V had no macro at all while x86 `-funsigned-char`
#      flipped the type and left the macro undefined);
#   3. *glibc's `<limits.h>`*, which computes `CHAR_MIN`/`CHAR_MAX` from that
#      macro — so row 1 of this gate is also a check that `<limits.h>` and
#      the type system cannot drift apart.
#
# The rows below are behavioural: a tiny program is compiled and RUN, and for
# the x86-64 rows GCC is compiled from the same source and its output must
# match byte for byte (GCC 14.2 here; the ABI facts it pins — signed char on
# x86-64, and the flag being order-sensitive within one command line — are
# stable across GCC versions).  The AArch64/RISC-V rows only need the
# preprocessor: the target selector reads the *binary name*, so the gate
# drives it through symlinks and preprocesses (no cross execution).
#
# Usage: bash tests/regression/check_char_signedness.sh
#        CCC=/path/to/lccc bash tests/regression/check_char_signedness.sh
set -eu

CCC=${CCC:-target/fastbuild/lccc}
if [ ! -x "$CCC" ]; then
  echo "SKIP  char-signedness (no compiler at $CCC)"
  exit 0
fi
CCC=$(cd "$(dirname "$CCC")" && pwd)/$(basename "$CCC")
GCC=$(command -v gcc || true)

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/probe.c" <<'EOF'
#include <limits.h>
#include <stdio.h>
int main(void)
{
    char c = (char)-1;
    printf("macro=%d min=%d max=%d neg=%d\n",
#ifdef __CHAR_UNSIGNED__
           1,
#else
           0,
#endif
           (int)CHAR_MIN, (int)CHAR_MAX, (int)(c < 0));
    return 0;
}
EOF

cat > "$TMP/signedness.c" <<'EOF'
#ifdef __CHAR_UNSIGNED__
unsigned_char
#else
signed_char
#endif
EOF

PASS=0
FAIL=0
note_ok()   { PASS=$((PASS + 1)); printf '  ok   %s  [%s]\n' "$1" "$2"; }
note_fail() { FAIL=$((FAIL + 1)); printf '  FAIL %s  [%s]\n' "$1" "$2"; }

# --- behavioural rows (x86-64: compile, run, compare with GCC) ---------------
# name | flags... | expected "macro min max neg"
behaviour_row() {
    local name=$1 want=$2
    shift 2
    local got
    if ! "$CCC" "$@" "$TMP/probe.c" -o "$TMP/a.out" 2>"$TMP/err"; then
        note_fail "$name" "lccc: $(head -1 "$TMP/err")"
        return
    fi
    got=$("$TMP/a.out")
    if [ "$got" != "$want" ]; then
        note_fail "$name" "lccc=$got want=$want"
        return
    fi
    if [ -n "$GCC" ]; then
        local grun
        if ! "$GCC" "$@" "$TMP/probe.c" -o "$TMP/g.out" 2>/dev/null; then
            note_fail "$name" "gcc rejected the same flags"
            return
        fi
        grun=$("$TMP/g.out")
        if [ "$grun" != "$got" ]; then
            note_fail "$name" "gcc=$grun lccc=$got"
            return
        fi
        note_ok "$name" "$got  (== gcc)"
    else
        note_ok "$name" "$got  (no gcc: oracle row skipped)"
    fi
}

behaviour_row "default (x86-64: signed char)"            "macro=0 min=-128 max=127 neg=1"
behaviour_row "-fsigned-char names the default"         "macro=0 min=-128 max=127 neg=1" -fsigned-char
behaviour_row "-funsigned-char"                         "macro=1 min=0 max=255 neg=0"    -funsigned-char
behaviour_row "-funsigned-char beats an earlier -fsigned-char" "macro=1 min=0 max=255 neg=0" -fsigned-char -funsigned-char
behaviour_row "-fsigned-char beats an earlier -funsigned-char" "macro=0 min=-128 max=127 neg=1" -funsigned-char -fsigned-char

# --- preprocessor rows (target defaults, selected by binary name) -----------
# The driver has no `-target` flag: the target comes from the binary name
# (`aarch64-`/`arm` -> AArch64, `riscv` -> Riscv64), so run through symlinks.
macro_row() {
    local name=$1 want=$2 bin=$3
    shift 3
    local out
    if ! out=$("$TMP/$bin" -E "$@" "$TMP/signedness.c" 2>"$TMP/err"); then
        note_fail "$name" "lccc: $(head -1 "$TMP/err")"
        return
    fi
    local got
    if printf '%s' "$out" | grep -q '^unsigned_char$'; then got=unsigned_char
    elif printf '%s' "$out" | grep -q '^signed_char$'; then got=signed_char
    else note_fail "$name" "no marker in preprocessed output"; return
    fi
    if [ "$got" = "$want" ]; then
        note_ok "$name" "c_unsigned=$([ "$got" = unsigned_char ] && echo 1 || echo 0)"
    else
        note_fail "$name" "got=$got want=$want"
    fi
}

ln -sf "$CCC" "$TMP/lccc"                     # x86-64 by the same rule
ln -sf "$CCC" "$TMP/aarch64-linux-gnu-lccc"
ln -sf "$CCC" "$TMP/riscv64-linux-gnu-lccc"

macro_row "aarch64 default (ABI: unsigned char)"  unsigned_char aarch64-linux-gnu-lccc
macro_row "riscv64 default (ABI: unsigned char)"  unsigned_char riscv64-linux-gnu-lccc
macro_row "aarch64 -fsigned-char"                 signed_char   aarch64-linux-gnu-lccc -fsigned-char
macro_row "riscv64 -fsigned-char"                 signed_char   riscv64-linux-gnu-lccc -fsigned-char
macro_row "x86-64 default"                        signed_char   lccc
macro_row "x86-64 -funsigned-char"                unsigned_char lccc -funsigned-char

echo
if [ "$FAIL" -eq 0 ]; then
    echo "char-signedness gate: $PASS passed, 0 failed"
else
    echo "char-signedness gate: $PASS passed, $FAIL FAILED"
    exit 1
fi
