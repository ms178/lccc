#!/usr/bin/env bash
# Verify that body-less enum references resolve through lexical tag scope and
# retain GCC-compatible signedness, while a genuinely incompatible pointer is
# still rejected. This is a compile/runtime contract, not a source-text check.
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-"$root/target/fastbuild/lccc"}
CC=${CC:-gcc}
[[ -x "$CCC" ]] || { echo "missing LCCC compiler: $CCC" >&2; exit 1; }
command -v "$CC" >/dev/null 2>&1 || { echo "missing C oracle: $CC" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for opt in -O0 -O2; do
    "$CCC" "$opt" "$root/tests/regression/enum_tag_scope_compat.c" -o "$work/lccc-${opt#-}"
    "$work/lccc-${opt#-}" >"$work/lccc-${opt#-}.out"
    "$CC" -Werror=incompatible-pointer-types "$opt" \
        "$root/tests/regression/enum_tag_scope_compat.c" -o "$work/oracle-${opt#-}"
    "$work/oracle-${opt#-}" >"$work/oracle-${opt#-}.out"
    cmp "$work/lccc-${opt#-}.out" "$work/oracle-${opt#-}.out"
    grep -Fxq 'outer enum definitions restored' "$work/lccc-${opt#-}.out"
done

# Keep the negative-control TU ephemeral: the broad regression corpus treats
# every checked-in C file as a positive compile case.
cat >"$work/incompatible-pointer.c" <<'EOF'
enum unsigned_tag { U = 1 };
static int read_int(int *p) { return *p; }
int main(void) {
    enum unsigned_tag value = U;
    return read_int(&value);
}
EOF
if "$CCC" -fsyntax-only "$work/incompatible-pointer.c" >"$work/lccc-negative.out" 2>&1; then
    echo 'LCCC accepted an incompatible enum/int pointer in the negative control' >&2
    exit 1
fi
if "$CC" -Werror=incompatible-pointer-types -fsyntax-only \
        "$work/incompatible-pointer.c" >"$work/oracle-negative.out" 2>&1; then
    echo "$CC accepted the incompatible enum/int pointer negative control" >&2
    exit 1
fi

echo 'enum tag scope compatibility: PASS (signed/unsigned shadowing at -O0/-O2; bad pointer rejected)'
