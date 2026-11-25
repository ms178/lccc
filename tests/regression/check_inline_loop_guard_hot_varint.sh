#!/usr/bin/env bash
# Structural companion to inline_loop_guard_hot_varint.c: execution alone
# cannot prove the atomic two-site -O2 clone or the -Os size-optimization gate.
set -euo pipefail

here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "loop-guard test: lccc not found at $CCC" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# If the compiler's IR verifier rejects a clone, compilation must fail too.
CCC_VERIFY_IR=abort "$CCC" -O2 -S "$here/inline_loop_guard_hot_varint.c" -o "$work/hot.s"
"$CCC" -Os -S "$here/inline_loop_guard_hot_varint.c" -o "$work/small.s"

# Count transfers to the decoder *within a single caller*, not the entire
# assembly. Jumps also represent out-of-line sibling calls.
transfers_to_decoder() { # transfers_to_decoder <asm> <caller>
    awk -v fn="$2" '
        $0 == fn ":" { inside = 1; next }
        inside && $1 == ".size" && $2 == fn "," { exit }
        inside { print }
    ' "$1" | grep -Ec '^[[:space:]]*(call|jmp)[[:space:]]+sqlite_get_varint([[:space:]]|$)' || true
}

for caller in hot_forward hot_backward; do
    hot=$(transfers_to_decoder "$work/hot.s" "$caller")
    small=$(transfers_to_decoder "$work/small.s" "$caller")
    if [[ $hot != 0 || $small != 1 ]]; then
        echo "loop-guard test: $caller has $hot decoder transfers at -O2 (want 0), $small at -Os (want 1)" >&2
        exit 1
    fi
done
# Neither an escaped decoder body nor a spurious emitted static stub should
# remain after both call sites have been cloned.
if grep -Eq '^sqlite_get_varint:' "$work/hot.s"; then
    echo 'loop-guard test: dead decoder still emitted at -O2' >&2
    exit 1
fi
if ! grep -Eq '^sqlite_get_varint:' "$work/small.s"; then
    echo 'loop-guard test: missing outlined decoder at -Os' >&2
    exit 1
fi
echo 'loop-guard test: both sites cloned at -O2, neither at -Os; dead body removed'
