#!/usr/bin/env bash
# Fast, fail-closed executable contracts distilled from the a9987d0 audit.
# No remote oracle, hardware feature assumptions, or unbounded hung programs.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
cc=${CC:-gcc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
python3 scripts/test_audit_levkropp_since.py
for name in swar_intermediate_casts count_zero_intermediate_casts cmp_select_fusion_stale_cond loop_memory_promote_alias_load; do
    src=tests/regression/$name.c
    timeout 60 "$cc" -O2 "$src" -o "$td/ref"
    timeout 10 "$td/ref" > "$td/expected"
    for opt in -O0 -O1 -O2 -O3; do
        timeout 60 "$ccc" "$opt" "$src" -o "$td/check"
        if ! timeout 10 "$td/check" > "$td/actual"; then
            echo "FAIL: $name $opt (nonzero exit or timeout)" >&2
            cat "$td/actual" >&2
            exit 1
        fi
        diff -u "$td/expected" "$td/actual"
    done
    echo "PASS: $name (-O0/-O1/-O2/-O3; executable checks + GCC stdout)"
done
# Compile-only ISA contract: no host POPCNT capability is needed. The
# positive canonical idiom must still fold; truncated stages must not.
timeout 60 "$ccc" -O2 -march=x86-64-v3 -S tests/regression/swar_intermediate_casts.c -o "$td/code.s"
python3 - "$td/code.s" <<'PY'
from pathlib import Path
import re
import sys
text = Path(sys.argv[1]).read_text()
for name in ("swar_full", "swar_low16", "swar_low8", "swar_stage1", "swar_stage2"):
    body = re.search(r"^" + name + r":\n(.*?)^\.size\s+" + name + r"\b", text, re.M | re.S)
    if body is None:
        raise SystemExit("FAIL: missing assembly body " + name)
    count = len(re.findall(r"^\s*popcnt\w*\s", body[1], re.M))
    if count != (1 if name == "swar_full" else 0):
        raise SystemExit("FAIL: unexpected POPCNT in " + name)
print("PASS: canonical POPCNT retained, no unsound truncated-stage folds")
PY
