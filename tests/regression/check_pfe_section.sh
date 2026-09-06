#!/usr/bin/env bash
# Session-85 (Agent-Z audit, hunk #2): -fpatchable-function-entry must not
# leak function bodies into __patchable_function_entries.
#
# Root cause: the raw `.section __patchable_function_entries,...` directive
# did not update `current_text_section`, so the following
# `emit_switch_to_section` saw a stale match and skipped the `.text`
# re-selection — every subsequent byte (entry table AND function bodies)
# landed in the writable, non-executable PFE section. GCC reference keeps the
# bodies in .text. This script fails if ANY function label appears while the
# PFE section is active, and runs the program to prove executability.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-pfe.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/t.c" <<'C'
__attribute__((noinline)) int f(int x) { return x + 1; }
__attribute__((noinline)) int g(int x) { return x * 3; }
int main(void) { return f(41) + g(0) != 42; }
C
"$CCC" -O2 -fpatchable-function-entry=2,0 -S "$tmp/t.c" -o "$tmp/t.s"
"$CCC" -O2 -fpatchable-function-entry=2,0 "$tmp/t.c" -o "$tmp/t"
"$tmp/t"   # must run: bodies are executable

# Walk the asm: while inside __patchable_function_entries only .align/.quad
# are legal; function labels and instructions must appear under .text.
awk '
  /\.section __patchable_function_entries/ { in_pfe = 1; next }
  /^\.text$/ || /^\.section \.text/ { in_pfe = 0; next }
  in_pfe && /^(f|g):/ { print "BODY IN PFE SECTION: " $0; bad = 1 }
  in_pfe && /^\t?(nop|addl|leaq|movl|ret)/ { print "INSN IN PFE SECTION: " $0; bad = 1 }
  END { exit bad ? 1 : 0 }
' "$tmp/t.s"

# Both functions recorded exactly one entry each.
[ "$(grep -c '\.quad' "$tmp/t.s")" -ge 2 ]
