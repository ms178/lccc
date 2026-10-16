#!/usr/bin/env bash
# A14-13: no LP64 width guesses for ext_vector_type in casts/compound literals.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
gcc_bin=${GCC:-gcc}
source "$root/tests/regression/i386_exec.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
src=$root/tests/regression/ext_vector_size.c

# A broken compiler must not turn a missing build into a successful gate.
[ -x "$ccc" ] || { echo "FAIL: compiler not built: $ccc" >&2; exit 1; }
for bits in 64 32; do
    if [[ $bits == 32 ]] && ! i386_run_or_skip "$gcc_bin"; then
        if [[ ${LCCC_REQUIRE_I686:-0} == 1 ]]; then exit 1; fi
        continue
    fi
    for opt in 0 2; do
        exe=$tmp/vector-$bits-$opt
        "$ccc" -m"$bits" -O"$opt" "$src" -o "$exe"
        if ! "$exe"; then
            echo "FAIL: ext_vector_type width -m$bits -O$opt" >&2
            exit 1
        fi
        echo "PASS: ext_vector_type width -m$bits -O$opt"
    done
done

# Invalid lane types and invalid counts must be diagnosed, not transformed
# into an apparently valid 4-byte vector (or a wrapped negative byte count).
for kind in void pointer zero negative runtime; do
    case "$kind" in
        void) source='int main(void) { return sizeof((void __attribute__((ext_vector_type(4)))){0}); }' ;;
        pointer) source='typedef int *ptr_t; int main(void) { return sizeof((ptr_t __attribute__((ext_vector_type(4)))){0}); }' ;;
        zero) source='int main(void) { return sizeof((long __attribute__((ext_vector_type(0)))){0}); }' ;;
        negative) source='int main(void) { return sizeof((long __attribute__((ext_vector_type(-1)))){0}); }' ;;
        runtime) source='int n; int main(void) { return sizeof((long __attribute__((ext_vector_type(n)))){0}); }' ;;
    esac
    printf '%s\n' "$source" > "$tmp/$kind.c"
    if "$ccc" -m64 -c "$tmp/$kind.c" -o "$tmp/$kind.o" >"$tmp/$kind.log" 2>&1; then
        echo "FAIL: ext_vector_type $kind accepted" >&2
        exit 1
    fi
    if ! grep -q 'ext_vector_type' "$tmp/$kind.log"; then
        echo "FAIL: ext_vector_type $kind rejected without attribute diagnostic" >&2
        cat "$tmp/$kind.log" >&2
        exit 1
    fi
    echo "PASS: ext_vector_type $kind diagnosed"
done
