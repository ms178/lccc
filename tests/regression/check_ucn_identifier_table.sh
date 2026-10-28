#!/usr/bin/env bash
# The characters a UCN (or a raw UTF-8 byte) may contribute to an identifier,
# and the diagnostics for the ones it may not, must match GCC 16.2 exactly.
#
# WHY: `src/frontend/lexer/ucn_id_table.rs` is a measured copy of the Annex D
# tables of C99/C11 -- not a Unicode property lookup. A Unicode-property
# approximation (`is_alphabetic`, `is_alphanumeric`, or "any non-whitespace code
# point") disagrees with GCC in both directions:
#
#   * `€` (U+20AC, Sc), U+200B (Cf), U+0301 (Mn) and emoji ARE identifier
#     characters (Annex D lists them), and
#   * U+00A9 (So), U+E000 and U+F0000 (private use) are NOT, even though every
#     one of them is a non-whitespace scalar value.
#
# This check re-measures a sample of that table against the oracle on every run,
# so the checked-in data cannot drift from the compiler it claims parity with.
# The full sweep (every code point in 0xA0..0x10FFFF, both positions) is the
# procedure recorded in the table's header.
#
# Usage: CCC=target/fastbuild/lccc GCC=gcc-16 bash check_ucn_identifier_table.sh
set -uo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
TMP=${TMPDIR:-/tmp}/lccc-ucnid.$$
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP"

if ! command -v "$GCC" >/dev/null 2>&1; then
    echo "check_ucn_identifier_table: SKIP (no $GCC oracle)"
    exit 0
fi
if [ ! -x "$CCC" ]; then
    echo "check_ucn_identifier_table: FAIL (no compiler at $CCC)" >&2
    exit 1
fi

fail=0
# veredict <compiler> <source-file>: prints ACCEPT or the first error message.
verdict() {
    local out
    out=$("$1" -fsyntax-only "$2" 2>&1)
    if [ $? -eq 0 ]; then
        echo "ACCEPT"
    else
        printf '%s\n' "$out" | sed -n 's/.*: error: //p' | head -1
    fi
}

# Parity over the interesting classes, in both identifier positions. Every row
# is <name> <utf-8 bytes as a printf escape>.
rows=(
    'euro:\xe2\x82\xac'          # Sc, in Annex D.1/D.2, not XID_Start
    'yend:\xc3\xbf'              # Latin-1 letter
    'eacute:\xc3\xa9'            # Ll, the everyday case
    'copyright:\xc2\xa9'         # So, NOT in Annex D: stray
    'nbsp:\xc2\xa0'              # Zs, NOT in Annex D: stray
    'zwsp:\xe2\x80\x8b'          # Cf, in D.1
    'combining:\xcc\x81'         # Mn, D.2 only: legal inside, not at the start
    'ideo_space:\xe3\x80\x80'    # Zs: stray
    'em_space:\xe2\x80\x83'      # Zs: stray
    'lycian:\xf0\x90\x80\x80'    # U+10000, in D.1
    'emoji:\xf0\x9f\x98\x80'     # U+1F600, in the 10000..1FFFD range
    'pua:\xee\x80\x80'           # U+E000 private use: stray
    'hangul:\xea\xb0\x80'        # U+AC00, in 3031..D7FF
    'plane16:\xef\xa0\x80'       # U+F0000 private use: stray
    'replacement:\xef\xbf\xbd'   # U+FFFD, in D.1
    'greek:\xce\xb1'             # U+03B1
)
for row in "${rows[@]}"; do
    name=${row%%:*}
    bytes=${row#*:}
    printf "int %bx = 1;\n" "$bytes" >"$TMP/start.c"
    printf "int a%bb = 1;\n" "$bytes" >"$TMP/cont.c"
    for pos in start cont; do
        want=$(verdict "$GCC" "$TMP/$pos.c")
        got=$(verdict "$CCC" "$TMP/$pos.c")
        if [ "$want" != "$got" ]; then
            echo "FAIL $name ($pos): gcc='$want' lccc='$got'" >&2
            fail=1
        fi
    done
done

# The UCN spellings are judged by the same tables, and the diagnostics carry
# GCC's exact wording, including which class the code point fell into.
while IFS='|' read -r name src want; do
    printf '%s\n' "$src" >"$TMP/ucn.c"
    got=$(verdict "$CCC" "$TMP/ucn.c")
    if [ "$got" != "$want" ]; then
        echo "FAIL $name: want='$want' got='$got'" >&2
        fail=1
    fi
    gcc_got=$(verdict "$GCC" "$TMP/ucn.c")
    if [ "$gcc_got" != "$want" ]; then
        echo "FAIL $name: oracle drifted, gcc='$gcc_got' (pinned '$want')" >&2
        fail=1
    fi
done <<'CASES'
ucn_below_a0|int a\u0041b = 1;|universal character \u0041 is not valid in an identifier
ucn_nbsp|int a\u00a0b = 1;|universal character \u00a0 is not valid in an identifier
ucn_copyright|int a\u00a9b = 1;|universal character \u00a9 is not valid in an identifier
ucn_pua|int a\uE000b = 1;|universal character \uE000 is not valid in an identifier
ucn_start_only|int \u0301x = 1;|universal character \u0301 is not valid at the start of an identifier
ucn_surrogate|int a\uD800b = 1;|\uD800 is not a valid universal character
ucn_above_unicode|int a\U00110000b = 1;|universal character \U00110000 is not valid in an identifier
ucn_above_31bit|int a\UFFFFFFFFb = 1;|\UFFFFFFFF is not a valid universal character
ucn_ok_euro|int a\u20ac = 1; int f(void){return a\u20ac;}|ACCEPT
ucn_ok_combining|int a\u0301 = 1;|ACCEPT
ucn_same_symbol|int caf\u00e9 = 1; int f(void){return caf\u00e9;}|ACCEPT
CASES

# Definition spelled with a UCN, use spelled with raw UTF-8, and vice versa:
# one identifier (C99 6.4.2.1), proven by the symbol table -- a mismatch shows up
# as an undefined `café` reference in the object file.
printf 'int caf\\u00e9 = 7;\nint use_ucn(void) { return caf\xc3\xa9; }\n' >"$TMP/id1.c"
printf 'int caf\xc3\xa9 = 7;\nint use_raw(void) { return caf\\U000000E9; }\n' >"$TMP/id2.c"
for idcase in id1 id2; do
    if ! "$CCC" -c "$TMP/$idcase.c" -o "$TMP/$idcase.o" 2>"$TMP/$idcase.err"; then
        echo "FAIL $idcase: $(head -1 "$TMP/$idcase.err")" >&2
        fail=1
        continue
    fi
    if nm "$TMP/$idcase.o" | grep -q ' U '; then
        echo "FAIL $idcase: UCN and UTF-8 spellings are different symbols:" >&2
        nm "$TMP/$idcase.o" >&2
        fail=1
    fi
    if ! nm "$TMP/$idcase.o" | grep -q "$(printf 'D caf\303\251')"; then
        echo "FAIL $idcase: definition is not spelled 'caf\xc3\xa9' in the object:" >&2
        nm "$TMP/$idcase.o" >&2
        fail=1
    fi
done

# Stray bytes use GCC's wording: printable ASCII as itself, everything else in
# octal. A byte of a non-UTF-8 file is reported as that byte, not as the lead
# byte of the private-use encoding it is carried in.
#
# Not covered: the bytes 0x01..0x04, which the preprocessor itself uses as
# blue-paint / paste-protection / token-boundary markers in expanded text and
# therefore strips from the source before the lexer sees them (a pre-existing
# limitation of that marker scheme, unrelated to identifier rules; GCC reports
# them as `stray '\1'`).
stray_case() { # stray_case <name> <printf-format-source> <expected>
    printf "$2" >"$TMP/stray.c"
    got=$(verdict "$CCC" "$TMP/stray.c")
    want=$3
    if [ "$got" != "$want" ]; then
        echo "FAIL $1: want='$want' got='$got'" >&2
        fail=1
    fi
    gcc_got=$(verdict "$GCC" "$TMP/stray.c")
    if [ "$gcc_got" != "$want" ]; then
        echo "FAIL $1: oracle drifted, gcc='$gcc_got' (pinned '$want')" >&2
        fail=1
    fi
}
stray_case at 'int a@b;\n' "stray '@' in program"
stray_case backtick 'int a`b;\n' "stray '\`' in program"
stray_case del 'int a\177b;\n' "stray '\177' in program"
stray_case raw_byte 'int a\377b;\n' "stray '\377' in program"
stray_case multibyte 'int a\xc2\xa9b;\n' "stray '\302' in program"

if [ "$fail" -ne 0 ]; then
    echo "check_ucn_identifier_table: FAILED" >&2
    exit 1
fi
echo "check_ucn_identifier_table: PASSED"
