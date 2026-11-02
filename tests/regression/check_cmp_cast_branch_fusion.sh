#!/usr/bin/env bash
# A C comparison promoted through multiple integer casts must feed jcc directly.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cat >"$td/test.c" <<'EOF'
int count_once(signed char x)
{
    int count = 0;
    while ((int)(signed char)(x <= 9)) {
        ++count;
        x = 10;
    }
    return count;
}
EOF
"$CCC" -m32 -Os -S "$td/test.c" -o "$td/test.s"
body=$(sed -n '/^count_once:/,/^\.size count_once/p' "$td/test.s")
if grep -Eq '\bset(le|be)\b|mov[sz]bl[[:space:]]+%al|testl[[:space:]]+%eax' <<<"$body"; then
    echo "comparison was materialized across the integer-cast chain"
    exit 1
fi
if ! grep -Eq '^[[:space:]]+j(g|a)[[:space:]]' <<<"$body"; then
    echo "direct fused comparison branch is absent"
    exit 1
fi
