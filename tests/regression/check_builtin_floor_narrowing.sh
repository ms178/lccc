#!/usr/bin/env bash
# Pin the floor(float) -> floorf(float) builtin contraction and its exact
# command-line boundary.  GCC 16.2 and Clang 23.1 agree on all rows checked
# here, including the non-obvious one: -fno-builtin-floorf does not disable a
# contraction enabled by the *floor* builtin, while -fno-builtin-floor does.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=$(realpath "${CCC:-$repo/target/fastbuild/lccc}")
CCC_I686=$(realpath "${CCC_I686:-$repo/target/fastbuild/lccc-i686}")
[[ -x $CCC ]] || { echo "missing x86-64 LCCC: $CCC" >&2; exit 1; }
[[ -x $CCC_I686 ]] || { echo "missing i686 LCCC: $CCC_I686" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cat >"$work/calls.c" <<'EOF'
double floor(double);
float floorf(float);
float plain_float(float x) { return floor(x); }
double plain_double(float x) { return floor(x); }
float explicit_cast(float x) { return floor((double)x); }
float builtin_float(float x) { return __builtin_floor(x); }
double builtin_double(float x) { return __builtin_floor(x); }
float double_arg(double x) { return floor(x); }
float builtin_double_arg(double x) { return __builtin_floor(x); }
EOF

body() { # body <assembly> <function>
    awk -v fn="$2" '
        $0 ~ "^" fn ":" { inside = 1 }
        inside { print }
        inside && $0 ~ "^\\.size[[:space:]]+" fn "," { exit }
    ' "$1"
}

has_call() { # has_call <function-body> <symbol>
    local text=$1 symbol=$2
    grep -Eq "(call|jmp)(q|l)?[[:space:]]+${symbol}(@PLT)?([[:space:]]|$)" <<<"$text"
}

require_call() { # require_call <label> <function-body> <wanted> <other>
    local label=$1 text=$2 wanted=$3 other=$4
    if ! has_call "$text" "$wanted"; then
        echo "FAIL: $label does not call $wanted" >&2
        printf '%s\n' "$text" >&2
        exit 1
    fi
    if has_call "$text" "$other"; then
        echo "FAIL: $label unexpectedly retains a $other call" >&2
        printf '%s\n' "$text" >&2
        exit 1
    fi
}

check_mode() { # check_mode <compiler> <arch> <mode> <plain-target> [flags...]
    local cc=$1 arch=$2 mode=$3 plain=$4
    shift 4
    local asm="$work/$arch-$mode.s"
    "$cc" -O2 "$@" -S "$work/calls.c" -o "$asm"
    local other=floor
    [[ $plain == floor ]] && other=floorf
    local fn text
    for fn in plain_float plain_double explicit_cast; do
        text=$(body "$asm" "$fn")
        [[ -n $text ]] || { echo "FAIL: missing $fn in $asm" >&2; exit 1; }
        require_call "$arch $mode $fn" "$text" "$plain" "$other"
    done
    for fn in builtin_float builtin_double; do
        text=$(body "$asm" "$fn")
        require_call "$arch $mode $fn" "$text" floorf floor
    done
    for fn in double_arg builtin_double_arg; do
        text=$(body "$asm" "$fn")
        require_call "$arch $mode $fn (precision guard)" "$text" floor floorf
    done
}

check_mode "$CCC" x86-64 default floorf
check_mode "$CCC" x86-64 no-floor floor -fno-builtin-floor
check_mode "$CCC" x86-64 no-floorf floorf -fno-builtin-floorf
check_mode "$CCC" x86-64 no-builtins floor -fno-builtin
check_mode "$CCC" x86-64 freestanding floor -ffreestanding

# i686 lowers the contracted float operation to inline x87 frndint code, so a
# call-name assertion would reject the better implementation.  Execute the
# original abort-on-floor() oracle on both targets instead; unlike an assembly
# pattern it also covers -O0, where the narrowing is already valid and useful.
runtime=$repo/tests/regression/builtin_floor_float_narrowing.c
for arch in x86-64 i686; do
    cc=$CCC
    [[ $arch == i686 ]] && cc=$CCC_I686
    for opt in -O0 -O1 -O2 -O3 -Os; do
        exe=$work/runtime-$arch$opt
        "$cc" "$opt" "$runtime" -o "$exe"
        timeout 10 "$exe"
        values=$work/values-$arch$opt
        "$cc" "$opt" "$repo/tests/regression/builtin_floor_float_values.c" -lm -o "$values"
        timeout 10 "$values"
    done
done

echo "builtin floor float narrowing: PASS (x86-64 call/flag contracts; x86-64 + i686 runtime at -O0..-Os)"
