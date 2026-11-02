#!/usr/bin/env bash
# Compile+run a slice of gcc.c-torture/execute with native lccc (x86-64).
#
# Usage:
#   scripts/x86_gcc_torture_slice.sh [test.c ...]
#   scripts/x86_gcc_torture_slice.sh --from-list FILE
#
# Environment:
#   LCCC          compiler binary (default: target/fastbuild/lccc)
#   GCC_TORTURE   execute/ directory
#   CCCFLAGS      extra lccc flags (default: -O2)
set -u
repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LCCC=${LCCC:-"$repo_root/target/fastbuild/lccc"}
GCC_TORTURE=${GCC_TORTURE:-/home/user/src/gcc/gcc/testsuite/gcc.c-torture/execute}
CCCFLAGS=${CCCFLAGS:--O2}

if [[ ! -x "$LCCC" ]]; then
    echo "error: lccc not found at $LCCC" >&2
    exit 2
fi

tests=()
if [[ "${1:-}" == "--from-list" ]]; then
    shift
    list=${1:?}
    shift
    while IFS= read -r line || [[ -n "$line" ]]; do
        [[ -z "$line" || "$line" == \#* ]] && continue
        tests+=("$line")
    done < "$list"
else
    tests=("$@")
fi

if [[ ${#tests[@]} -eq 0 ]]; then
    echo "usage: $0 test.c ... | $0 --from-list FILE" >&2
    exit 2
fi

pass=0
fail=0
compile_fail=0
for t in "${tests[@]}"; do
    base=$(basename "$t")
    src=$t
    if [[ ! -f "$src" ]]; then
        src="$GCC_TORTURE/$base"
    fi
    if [[ ! -f "$src" ]]; then
        echo "MISSING $base"
        fail=$((fail + 1))
        continue
    fi
    out=/tmp/lccc_torture_${base%.c}
    err=/tmp/lccc_torture_${base%.c}.err
    if ! "$LCCC" $CCCFLAGS "$src" -o "$out" >"$err" 2>&1; then
        echo "COMPILE-FAIL $base"
        sed -n '1,8p' "$err" | sed 's/^/  /'
        compile_fail=$((compile_fail + 1))
        fail=$((fail + 1))
        continue
    fi
    if "$out" >/dev/null 2>&1; then
        echo "PASS $base"
        pass=$((pass + 1))
    else
        rc=$?
        echo "FAIL $base (exit $rc)"
        fail=$((fail + 1))
    fi
done

echo "-- $pass pass / $fail fail ($compile_fail compile-fail) --"
[[ $fail -eq 0 ]]
