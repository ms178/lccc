#!/usr/bin/env bash
# Named/generic pointer conversion verdicts. GCC 13 rejects two VALID static
# member-array initializers as nonconstant; use the GCC >= 14 oracle explicitly
# provisioned by CI, not the runner's unversioned gcc. Never skip the oracle.
set -euo pipefail
export LC_ALL=C
CCC=${CCC:-./target/fastbuild/lccc}
dir=tests/regression/seg_conv
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-seg-conv.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

if [[ -n ${GCC:-} ]]; then
    candidates=("$GCC")
else
    # Prefer the newest installed supported oracle; selecting gcc-14 first
    # silently left the gate on an older compiler even when GCC 16 was present.
    candidates=(gcc-16 gcc-15 gcc-14 gcc)
fi
GCC=''
for candidate in "${candidates[@]}"; do
    command -v "$candidate" >/dev/null 2>&1 || continue
    version=$("$candidate" -dumpversion 2>/dev/null) || continue
    major=${version%%.*}
    if [[ $major =~ ^[0-9]+$ ]] && (( major >= 14 )); then
        GCC=$candidate
        break
    fi
done
if [[ -z $GCC ]]; then
    echo "FAIL: conversion oracle requires GCC >= 14; install gcc-16 (preferred) or set GCC to a suitable compiler (tried: ${candidates[*]})" >&2
    exit 1
fi
version=$("$GCC" --version)
echo "Conversion oracle: $GCC (${version%%$'\n'*})"
# Semantic positive control, not just a trivial TU: GCC 13's failure is in
# this exact constant-address form. A broken compiler/toolchain must not make
# negative fixtures look like valid rejections.
cat > "$tmp/control.c" <<'C'
struct S { long a[3]; };
extern struct S __seg_gs object;
long __seg_gs *p = object.a + 1;
C
for compiler in "$CCC" "$GCC"; do
    if ! "$compiler" -std=gnu11 -c "$tmp/control.c" -o "$tmp/control.o" >"$tmp/control.err" 2>&1; then
        echo "FAIL: $compiler failed the named-space positive control" >&2
        cat "$tmp/control.err" >&2
        exit 1
    fi
done

fail=0
shopt -s nullglob
files=("$dir"/neg_*.c "$dir"/pos_*.c)
if [[ ${#files[@]} -eq 0 ]]; then
    echo "FAIL: no fixtures in $dir" >&2
    exit 1
fi
for f in "${files[@]}"; do
    base=${f##*/}
    case "$base" in
        neg_*) want=reject ;;
        pos_*) want=accept ;;
    esac
    for compiler in "$CCC" "$GCC"; do
        if "$compiler" -std=gnu11 -c -o "$tmp/x.o" "$f" >"$tmp/diagnostic" 2>&1; then
            got=accept
        else
            status=$?
            got=reject
            if [[ $status -ne 1 ]]; then
                echo "FAIL: $compiler failed abnormally on $base (status $status)" >&2
                cat "$tmp/diagnostic" >&2
                fail=1
                continue
            fi
        fi
        if [[ $got != "$want" ]]; then
            echo "FAIL: $compiler $got $base (expected $want)" >&2
            cat "$tmp/diagnostic" >&2
            fail=1
        elif [[ $want = reject ]] && ! grep -q -e 'address space' -e 'incompatible pointer type for argument' "$tmp/diagnostic"; then
            echo "FAIL: $compiler rejects $base without an address-space diagnostic" >&2
            cat "$tmp/diagnostic" >&2
            fail=1
        fi
    done
done
if [[ $fail -ne 0 ]]; then
    echo 'check_seg_conv_diagnostics: FAILED' >&2
    exit 1
fi
echo "check_seg_conv_diagnostics: PASSED (${#files[@]} files, two compilers, positive controls)"
