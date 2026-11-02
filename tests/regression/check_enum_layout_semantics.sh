#!/usr/bin/env bash
# Enum layout semantics end-to-end: GCC-compatible widening (value range →
# unsigned int / long long), packed minimum width, struct-layout inheritance,
# and _Static_assert conditions evaluated by sema against that same layout.
# A false condition must be REJECTED (gcc rejects it; lccc used to accept).
set -euo pipefail

root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-"$root/target/fastbuild/lccc"}
CC=${CC:-gcc}
[[ -x "$CCC" ]] || { echo "missing LCCC compiler: $CCC" >&2; exit 1; }
command -v "$CC" >/dev/null 2>&1 || { echo "missing C oracle: $CC" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for opt in -O0 -O2; do
    "$CCC" "$opt" "$root/tests/regression/enum_layout_widening.c" -o "$work/lccc-${opt#-}"
    "$work/lccc-${opt#-}" >"$work/lccc-${opt#-}.out"
    "$CC" "$opt" "$root/tests/regression/enum_layout_widening.c" -o "$work/oracle-${opt#-}"
    "$work/oracle-${opt#-}" >"$work/oracle-${opt#-}.out"
    cmp "$work/lccc-${opt#-}.out" "$work/oracle-${opt#-}.out"

    "$CCC" "$opt" "$root/tests/regression/enum_static_assert_ice.c" -o "$work/assert-${opt#-}"
done

# Negative controls (ephemeral): a false static assertion must be a compile
# ERROR, and this must agree with the oracle.  Kept out of the corpus: the
# broad regression corpus treats every checked-in C file as compilable.
cat >"$work/assert_false_size.c" <<'PINEOF'
enum Big { B0 = 0x100000000LL };
_Static_assert(sizeof(enum Big) == 4, "sema says 4");
int main(void) { return 0; }
PINEOF
cat >"$work/assert_literal_false.c" <<'PINEOF'
_Static_assert(0, "must fail");
int main(void) { return 0; }
PINEOF
cat >"$work/assert_bad_builtin_size.c" <<'PINEOF'
_Static_assert(sizeof(int) == 5, "int is not 5");
int main(void) { return 0; }
PINEOF
cat >"$work/assert_var_ref.c" <<'PINEOF'
int f(void) { int x = 1; _Static_assert(x, "not an ICE"); return x; }
PINEOF
for neg in assert_false_size assert_literal_false assert_bad_builtin_size; do
    if "$CCC" -fsyntax-only "$work/$neg.c" >"$work/lccc-$neg.out" 2>&1; then
        echo "LCCC accepted a false static assertion ($neg)" >&2
        exit 1
    fi
    if "$CC" -fsyntax-only "$work/$neg.c" >"$work/oracle-$neg.out" 2>&1; then
        echo "$CC accepted a false static assertion ($neg)" >&2
        exit 1
    fi
done
# Variable references are diagnosed by the parser as non-ICE (C11 6.6).
if "$CCC" -fsyntax-only "$work/assert_var_ref.c" >"$work/lccc-var.out" 2>&1; then
    echo "LCCC accepted a variable reference in a static assertion" >&2
    exit 1
fi

echo 'enum layout semantics: PASS (GCC-identical widening/packed/struct layouts at -O0/-O2; false assertions rejected)'
