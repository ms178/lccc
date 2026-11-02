#!/usr/bin/env bash
# ============================================================================
# elf_sections.sh — shared measurement helpers for the boot-size harnesses:
# executable bytes per object (summed by section FLAG) and the delta ranking the
# oracle prints.  Both are here because both were wrong in a way nothing else
# could catch: the harnesses only run against a prepared kernel tree, so a
# silent measurement error survives every CI gate.  tests/regression/
# check_boot_size_measurement.sh pins both without needing a kernel tree.
#
# Sourced, never executed:  . "$(dirname "$0")/elf_sections.sh"
#
# Why this module exists: boot_size_oracle.sh and realmode_corpus.sh each summed
# executable bytes with a NAME pattern (`size -A | $1 ~ /^\.text/`) and both
# under-reported, because the boot stage keeps code outside `.text`:
#
#   header.o     .bstext (408 B) + .entrytext (104 B)  -> reported as 0 bytes
#   bioscall.o   .inittext (95 B)                      -> reported as 0 bytes
#   tty.o        .inittext (311 B)                     -> reported as 321 of 632
#
# Over the 24 objects arch/x86/boot links into setup.elf that hid 918 of lccc's
# 25299 executable bytes (791 of gcc's 14270, 815 of clang's 15555).  The
# hidden part is not the same size in every compiler, so it also distorted the
# per-object ranking boot_size_oracle.sh prints "largest lccc excess first, the
# objects worth optimizing bubble to the top": tty.o's real gap against gcc is
# +184 bytes, not the +57 the name pattern produced.
#
# realmode_corpus.sh had already met this class once and widened `.text` to
# `.text.*` after GCC's -freorder-functions split main.c (its comment: "counting
# only .text silently underreported GCC (main.c read as 12 bytes instead of
# 493)").  No name list closes the class, because the kernel may put code in any
# SHF_EXECINSTR section it likes (.irqentry.text and friends already exist
# elsewhere in the tree).  The flag is the definition of code, so read the flag.
# ============================================================================

# lccc_elf_code_bytes <object> -> total bytes of every SHF_EXECINSTR section.
#
# `objdump -h` prints each section's flags on the line FOLLOWING its header and
# prints sizes in hex; mawk has no strtonum, so convert by hand.  An object that
# cannot be read yields 0 (the caller's arithmetic keeps working) while the
# pipeline's exit status still reports the failure to `set -o pipefail` callers.
lccc_elf_code_bytes() {
  objdump -h "$1" 2>/dev/null | awk '
    function hex2dec(h,   i, v) {
      v = 0; h = tolower(h); sub(/^0x/, "", h)
      for (i = 1; i <= length(h); i++)
        v = v * 16 + index("0123456789abcdef", substr(h, i, 1)) - 1
      return v
    }
    /^ *[0-9]+ *\./ { sz = $3; getline flags; if (flags ~ /CODE/) s += hex2dec(sz) }
    END { printf "%d\n", s + 0 }'
}

# lccc_rank_size_rows — stdin: the harness's "<object> <lccc> <oracle> <delta>
# [note]" rows; stdout: the same rows, largest delta first.
#
# `sort -n` does NOT parse a leading '+'.  GNU coreutils' numeric compare skips
# blanks and an optional '-' and then stops, so a `%+8d` delta such as "+1598"
# compares as 0: every row ties and sort falls back to its last-resort
# whole-line comparison, which -r reverses.  The table therefore came out in
# reverse-lexicographic order BY OBJECT NAME while claiming to be ranked by
# excess — measured on the 24 boot objects: video-vga +176 printed above printf
# +1598, and the one object the ranking existed to surface sat seventh.
#
# Sort on a sign-free key derived from the delta instead of asking -n to read
# the display column, keep the '+' in the printed row, and use -s so ties stay
# in link order (deterministic, and link order is the meaningful secondary).
lccc_rank_size_rows() {
  awk '{ printf "%d\t%s\n", $4, $0 }' \
    | sort -t"$(printf '\t')" -k1,1 -n -r -s \
    | cut -f2-
}
