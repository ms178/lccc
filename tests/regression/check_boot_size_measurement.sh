#!/usr/bin/env bash
# Regression coverage for scripts/elf_sections.sh — the two numbers the boot-size
# harnesses report.  Neither check needs a kernel tree, so both run in CI; the
# harnesses themselves only run against a prepared tree, which is exactly why
# both errors survived: a measurement bug out there is invisible to every gate.
#
# 1. Executable bytes must be summed by SECTION FLAG.  arch/x86/boot keeps code
#    outside `.text`: header.o in `.bstext`/`.entrytext`, bioscall.o and tty.o in
#    `.inittext`.  Summing `/^\.text/` reported header.o and bioscall.o as 0
#    bytes, hid 918 of lccc's 25299 executable bytes (791 of gcc's 14270, 815 of
#    clang's 15555) and understated tty.o's gap against gcc from +184 to +57.
#
# 2. The per-object ranking must be ordered by DELTA.  `sort -n` does not parse a
#    leading '+' (coreutils' numeric compare skips blanks and an optional '-' and
#    stops), so sorting the printed `%+8d` column tied every row at zero and sort
#    fell back to its last-resort whole-line comparison, reversed by -r: the
#    table came out in reverse-lexicographic order BY OBJECT NAME, printing
#    video-vga +176 above printf +1598.
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
# shellcheck source=../../scripts/elf_sections.sh
source "$repo_root/scripts/elf_sections.sh"

CC=${CC:-gcc}
fail=0
say_fail() { printf 'FAIL %s\n' "$*" >&2; fail=1; }
expect_eq() { # expect_eq <label> <actual> <expected>
    if [[ $2 != "$3" ]]; then say_fail "$1: got '$2', want '$3'"
    else printf 'ok   %s\n' "$1"; fi
}
expect_gt0() { # expect_gt0 <label> <value>
    if [[ $2 =~ ^[0-9]+$ ]] && (( $2 > 0 )); then printf 'ok   %s\n' "$1"
    else say_fail "$1: got '$2', want a positive integer"; fi
}

for tool in "$CC" readelf size objdump; do
    command -v "$tool" >/dev/null 2>&1 || {
        printf 'check_boot_size_measurement: %s not found\n' "$tool" >&2
        exit 1
    }
done

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# An INDEPENDENT measure of the same quantity: readelf's own section table,
# summed over sections whose flags carry X (SHF_EXECINSTR).  A different tool, a
# different parser, a different flag spelling -- agreement is evidence, not
# tautology.  (readelf leaves the Flg column blank for flagless sections, so $8
# holds Lk there; Lk/Inf/Al are decimal and can never contain an X.)
readelf_x_bytes() {
    readelf -SW "$1" | sed -E 's/\[ *([0-9]+)\]/[\1]/' | awk '
        function hex2dec(h,   i, v) {
            v = 0; h = tolower(h)
            for (i = 1; i <= length(h); i++)
                v = v * 16 + index("0123456789abcdef", substr(h, i, 1)) - 1
            return v
        }
        $2 ~ /^\./ && $8 ~ /X/ { s += hex2dec($6) } END { printf "%d\n", s + 0 }'
}

# The measure elf_sections.sh replaced, kept here so the regression is pinned in
# the direction it actually broke: for a non-.text code section the old sum is 0
# and the new one is not.
naive_text_bytes() {
    size -A "$1" 2>/dev/null | awk '
        $1 ~ /^\.text/ && $2 ~ /^[0-9]+$/ { s += $2 } END { printf "%d\n", s + 0 }'
}

printf '\n== 1. executable bytes are summed by section flag ==\n'

# Every name the kernel actually uses, plus one invented name: the rule is the
# flag, so no name may be special.
i=0
for sec in .text .text.unlikely .bstext .entrytext .inittext .lccc_invented_code; do
    i=$((i + 1))
    cat > "$tmp/one$i.c" <<EOF
__attribute__((section("$sec"))) int only_fn(int x) { return x * 3 + $i; }
EOF
    "$CC" -O2 -c "$tmp/one$i.c" -o "$tmp/one$i.o"
    got=$(lccc_elf_code_bytes "$tmp/one$i.o")
    expect_eq "code bytes in $sec match readelf's X-flagged sum" \
              "$got" "$(readelf_x_bytes "$tmp/one$i.o")"
    expect_gt0 "code bytes in $sec are counted" "$got"
    case $sec in
        .text*) expect_eq "naive /^\\.text/ sum already saw $sec" \
                          "$(naive_text_bytes "$tmp/one$i.o")" "$got" ;;
        *)      expect_eq "naive /^\\.text/ sum missed $sec entirely" \
                          "$(naive_text_bytes "$tmp/one$i.o")" 0 ;;
    esac
done

# The real-world shape: one object holding code in four sections plus data, bss
# and rodata that must NOT be counted.
cat > "$tmp/multi.c" <<'EOF'
__attribute__((section(".bstext")))    int boot_header(int x) { return x + 1; }
__attribute__((section(".entrytext"))) int boot_entry(int x)  { return x + 2; }
__attribute__((section(".inittext")))  int boot_init(int x)   { return x + 3; }
                                       int boot_plain(int x)  { return x + 4; }
int boot_data = 42;
const char boot_rodata[] = "not code, must not be counted";
EOF
"$CC" -O2 -c "$tmp/multi.c" -o "$tmp/multi.o"
multi=$(lccc_elf_code_bytes "$tmp/multi.o")
expect_eq "mixed-section object matches readelf's X-flagged sum" \
          "$multi" "$(readelf_x_bytes "$tmp/multi.o")"
naive_multi=$(naive_text_bytes "$tmp/multi.o")
if (( multi > naive_multi )); then
    printf 'ok   mixed-section object: the old name sum saw %d of %d code bytes\n' \
           "$naive_multi" "$multi"
else
    say_fail "mixed-section object: flag sum $multi is not greater than name sum $naive_multi"
fi

# Data only: nothing is executable, so the answer is zero, not "the size of
# .data" and not an error.
printf 'int only_data = 1;\nconst char only_ro[] = "ro";\n' > "$tmp/dataonly.c"
"$CC" -O2 -c "$tmp/dataonly.c" -o "$tmp/dataonly.o"
expect_eq "data-only object has no executable bytes" \
          "$(lccc_elf_code_bytes "$tmp/dataonly.o")" 0
expect_eq "data-only object agrees with readelf" \
          "$(readelf_x_bytes "$tmp/dataonly.o")" 0

# A missing object is a zero, not a crash: the harness sums these in arithmetic.
expect_eq "unreadable object reports 0" "$(lccc_elf_code_bytes "$tmp/nope.o")" 0

printf '\n== 2. the size table is ranked by delta ==\n'

# The exact row format boot_size_oracle.sh prints, and the exact shape of the
# failure it had: signed deltas, ties, one negative, and object names whose
# lexicographic order differs from the delta order.
rows=$(cat <<'EOF'
video-vga      1239     1063     +176   lccc larger
printf         3411     1813    +1598   lccc larger
version           0        0       +0
tty             632      448     +184   lccc larger
bioscall         95       95       +0
string         2209     1182    +1027   lccc larger
regs            117       55      +62   lccc larger
tty_cold        300      400     -100   lccc smaller
EOF
)
want_order='printf
string
tty
video-vga
regs
version
bioscall
tty_cold'
got_order=$(printf '%s\n' "$rows" | lccc_rank_size_rows | awk '{print $1}')
expect_eq "rows come out largest-delta-first (ties keep input order)" \
          "$got_order" "$want_order"
expect_eq "the signed delta column survives the ranking unchanged" \
          "$(printf '%s\n' "$rows" | lccc_rank_size_rows | head -1 | awk '{print $4}')" "+1598"

# The reverted pipeline must produce something DIFFERENT, otherwise this test
# would pass with the bug back in place.  `sort -k4 -n -r` reads every "+N" as 0.
buggy_order=$(printf '%s\n' "$rows" | sort -k4 -n -r | awk '{print $1}')
if [[ $buggy_order == "$got_order" ]]; then
    say_fail "the buggy pipeline reproduced the fixed order; this test cannot catch a revert"
else
    printf 'ok   the reverted pipeline is detectably wrong (its top row: %s)\n' \
           "$(printf '%s\n' "$buggy_order" | head -1)"
fi

# Row count and content must survive: ranking reorders, it never drops or edits.
expect_eq "ranking preserves every row" \
          "$(printf '%s\n' "$rows" | lccc_rank_size_rows | wc -l | tr -d '[:space:]')" \
          "$(printf '%s\n' "$rows" | wc -l | tr -d '[:space:]')"
expect_eq "ranking preserves row content as a multiset" \
          "$(printf '%s\n' "$rows" | lccc_rank_size_rows | sort)" \
          "$(printf '%s\n' "$rows" | sort)"

if (( fail != 0 )); then
    echo "check_boot_size_measurement: FAILED" >&2
    exit 1
fi
echo "check_boot_size_measurement: PASS (code bytes by flag, ranking by delta)"
