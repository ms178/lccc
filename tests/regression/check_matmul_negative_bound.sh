#!/usr/bin/env bash
# Signed negative bounds must not become packed memory trips in either FMA
# matmul lowering. The runtime fixture straddles the byte-bound overflow
# boundaries for I32 and I64; assembly assertions prove both transform widths
# remain active rather than letting a scalar fallback pass vacuously.
set -euo pipefail

CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
GCC=${GCC_BIN:-gcc}
source_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source="$source_dir/vectorize_matmul_negative_bound.c"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-matmul-negative-bound.XXXXXX")

# TEMP-DIAG (remove before merge): surface the failure reason as a GitHub
# annotation, because job logs are not readable while this gate is red.
diag_on_exit() { # diag_on_exit <exit-status>
    local rc=$1
    local f
    printf '::error title=matmul-gate::rc=%d bound_type=%s mode=%s\n' \
        "$rc" "${bound_type:-none}" "${mode:-none}"
    for f in "$tmp"/*.s; do
        [[ -f $f ]] || continue
        python3 - "$f" <<'PY2'
import re
import sys

text = open(sys.argv[1], encoding="utf-8", errors="replace").read()
match = re.search(r"(?ms)^matmul_vector_loop:.*?^\.size matmul_vector_loop,.*?$", text)
body = match.group(0) if match else "MISSING matmul_vector_loop symbol"
body = body[:6000].replace("%", "%25").replace("\n", "%0A").replace("\r", "")
print(f"::error title=asm-{sys.argv[1].rsplit('/', 1)[-1]}::{body}")
PY2
    done
    rm -rf "$tmp"
}
on_exit() {
    local status=$?
    diag_on_exit "$status"
}
trap on_exit EXIT

check_assembly() { # assembly path, mode, bound type
    python3 - "$1" "$2" "$3" <<'PY'
import re
import sys
from pathlib import Path

path, mode, bound_type = sys.argv[1:]
text = Path(path).read_text(encoding="utf-8")
match = re.search(r"(?ms)^matmul_vector_loop:\n(.*?)^\.size matmul_vector_loop,", text)
if not match:
    raise SystemExit(f"{path}: missing matmul_vector_loop assembly for {bound_type}/{mode}")
body = match.group(1)
packed = re.search(r"\b(?:v?fmadd[a-z0-9]*pd|v?mul(?:pd|upd)|v?addpd)\b", body)
if not packed:
    raise SystemExit(f"{path}: {bound_type}/{mode} fell back to scalar arithmetic")
if mode == "avx2":
    if "%ymm" not in body:
        raise SystemExit(f"{path}: {bound_type}/AVX2 lacks a YMM packed loop")
    if not re.search(r"(?m)^\s*shrq\s+\$3,\s*%r\w+", body):
        raise SystemExit(f"{path}: {bound_type}/AVX2 lacks a 64-bit byte-to-element shift")
    if bound_type == "i32" and not re.search(
        r"(?m)^\s*shrq\s+\$3,\s*%r\w+\n\s*movslq\s+%e\w+,\s*%r\w+",
        body,
    ):
        raise SystemExit(
            f"{path}: I32 AVX2 narrows the byte IV before computing the scalar tail"
        )
else:
    if "%xmm" not in body:
        raise SystemExit(f"{path}: {bound_type}/two-wide lacks an XMM packed loop")
    if "%ymm" in body:
        raise SystemExit(f"{path}: {bound_type}/two-wide unexpectedly uses YMM")
PY
}

run_ok() { # binary path, label, expected stdout
    local output
    output=$(timeout 10 "$1")
    if [[ "$output" != "OK" ]]; then
        printf 'FAIL: %s returned %q, expected OK\n' "$2" "$output" >&2
        return 1
    fi
}

for bound_type in i32 i64; do
    define=()
    if [[ "$bound_type" == i64 ]]; then
        define=(-DTEST_I64_BOUND)
    fi

    ref="$tmp/gcc-$bound_type"
    "$GCC" -O2 -march=x86-64-v3 "${define[@]}" "$source" -o "$ref"
    run_ok "$ref" "GCC $bound_type oracle"

    for mode in avx2 two-wide; do
        asm="$tmp/$bound_type-$mode.s"
        binary="$tmp/$bound_type-$mode"
        flags=(-O2)
        if [[ "$mode" == avx2 ]]; then
            flags+=(-march=x86-64-v3)
            "$CCC" "${flags[@]}" "${define[@]}" -S "$source" -o "$asm"
            "$CCC" "${flags[@]}" "${define[@]}" "$source" -o "$binary"
        else
            flags+=(-march=x86-64)
            env LCCC_FORCE_SSE2=1 "$CCC" "${flags[@]}" "${define[@]}" \
                -S "$source" -o "$asm"
            env LCCC_FORCE_SSE2=1 "$CCC" "${flags[@]}" "${define[@]}" \
                "$source" -o "$binary"
        fi

        check_assembly "$asm" "$mode" "$bound_type"
        run_ok "$binary" "LCCC $bound_type/$mode"
        actual=$(timeout 10 "$binary")
        expected=$(timeout 10 "$ref")
        if [[ "$actual" != "$expected" ]]; then
            printf 'FAIL: %s/%s output differs from GCC (%q vs %q)\n' \
                "$bound_type" "$mode" "$actual" "$expected" >&2
            exit 1
        fi
    done
done

printf 'signed matmul bounds: PASS (I32/I64, AVX2/two-wide, negative overflow boundaries)\n'
