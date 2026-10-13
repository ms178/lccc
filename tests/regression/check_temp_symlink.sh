#!/usr/bin/env bash
# AUD-TMP-1: mutation-sensitive reproduction of the old predictable-name attack.
set -euo pipefail
CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
mkdir "$td/tmpd"
printf 'VICTIM\n' > "$td/victim"
cp "$td/victim" "$td/expected"
for i in $(seq 1 400); do printf 'int f%s(int x){return x*%s;}\n' "$i" "$i"; done > "$td/big.c"
printf 'int main(void){return f1(0);}\n' >> "$td/big.c"
# exec keeps the known PID, so this is deterministic, not a race lottery.
CCC="$CCC" TD="$td" bash -c '
    ln -s "$TD/victim" "$TD/tmpd/ccc_$$_big.0.o"
    TMPDIR="$TD/tmpd" exec "$CCC" -O1 "$TD/big.c" -o "$TD/program"
'
cmp "$td/victim" "$td/expected"
"$td/program"
# Normal cleanup leaves only the attacker-owned symlink, never our directories.
if [ -n "$(find "$td/tmpd" -mindepth 1 -type d)" ]; then
    echo 'FAIL: temporary directories leaked' >&2; exit 1
fi
# Invalid TMPDIR must report a normal driver error, not an ICE or panic.
printf 'not a directory\n' > "$td/notdir"
if TMPDIR="$td/notdir" "$CCC" "$td/big.c" -o "$td/bad" >"$td/out" 2>"$td/err"; then
    echo 'FAIL: accepted non-directory TMPDIR' >&2; exit 1
fi
grep -q 'Failed to reserve temporary file' "$td/err"
if grep -Eq 'internal error|panicked' "$td/err"; then cat "$td/err"; exit 1; fi
# Debug dumps must stay in private TMPDIR storage and report their location.
printf '.text\n.globl temp_probe\ntemp_probe: ret\n' > "$td/probe.s"
TMPDIR="$td/tmpd" CCC_ASM_DEBUG=1 "$CCC" -c "$td/probe.s" -o "$td/probe.o" 2>"$td/debug"
grep -q "Assembly debug dump: $td/tmpd/lccc_" "$td/debug"
test "$(find "$td/tmpd" -type f -name 'asm_debug_probe.s' | wc -l)" = 1
printf 'temporary symlink isolation: PASS\n'
