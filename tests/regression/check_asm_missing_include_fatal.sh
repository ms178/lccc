#!/usr/bin/env bash
# A missing #include in a .S file must be FATAL (non-zero exit, diagnostic),
# exactly like GAS/cpp.  Before this fix the assembly preprocess path dropped
# preprocessor errors on the floor: the kernel's arch/x86/boot/header.S
# compiled happily with an EMPTY voffset.h, silently evaluating the VO_*
# `#if` guards to 0 and emitting a subtly wrong setup header.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

printf '#include "lccc_no_such_header_xyz.h"\n.text\nnop\n' > "$td/missing.S"
if "$CCC" -c "$td/missing.S" -o "$td/missing.o" 2>"$td/err.txt"; then
    echo "missing include in .S accepted (must be fatal)"
    exit 1
fi
grep -q "lccc_no_such_header_xyz.h" "$td/err.txt" || {
    echo "diagnostic does not name the missing header"; cat "$td/err.txt"; exit 1;
}
[ ! -e "$td/missing.o" ] || { echo "object emitted despite the error"; exit 1; }

# Sanity: a resolvable include still assembles fine.
printf '#define VALUE 42\n' > "$td/ok.h"
printf '#include "ok.h"\n.text\n.byte VALUE\n' > "$td/ok.S"
"$CCC" -c "$td/ok.S" -o "$td/ok.o" || { echo "resolvable include rejected"; exit 1; }
