#!/usr/bin/env bash
# A same-width U32<->I32 cast (a bit-preserving no-op on i686) must coalesce
# with its source: `cpu_vendor[0] == 'Genu'` (u32 global compared as I32) must
# NOT materialize a relay copy (`movl sym,%edx; movl %edx,%ebx; cmpl ...,%ebx`).
# Regression for the no-op-cast relay that dominated global-load compare sites.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cat >"$td/test.c" <<'EOF'
static unsigned vendor[4];
int is_magic(void)
{
    return vendor[0] == 0x47656e75u &&
           vendor[1] == 0x696e6549u &&
           vendor[2] == 0x36584d4du;
}
EOF
"$CCC" -m32 -Os -fno-pic -S "$td/test.c" -o "$td/test.s"
body=$(sed -n '/^is_magic:/,/^\.size is_magic/p' "$td/test.s")
# Each `movl vendor..., %REG` must be followed (ignoring labels) by
# `cmpl $imm, %REG` with the SAME register — the cast must not materialize a
# relay through a second register.
loads=$(grep -E 'movl[[:space:]]+vendor' <<<"$body" | sed -E 's/.*(%e[a-z]+)$/\1/')
cmps=$(grep -E 'cmpl[[:space:]]+\$[0-9]+,.*%e[a-z]+' <<<"$body" | sed -E 's/.*(%e[a-z]+)$/\1/')
if [ "$loads" != "$cmps" ]; then
    echo "global load registers and compare registers differ (relay present)"
    echo "loads: $loads"
    echo "cmps:  $cmps"
    echo "--- $body"
    exit 1
fi
if [ "$(echo "$loads" | wc -l)" -ne 3 ]; then
    echo "expected 3 global compare sites, got $(echo "$loads" | wc -l)"
    echo "--- $body"
    exit 1
fi
