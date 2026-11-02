#!/usr/bin/env bash
# F1 contract (red-team audit): under -fno-builtin / -ffreestanding, the
# BARE library spellings must not fold in constant initializers. Before the
# prefix gate in `eval_builtin_call_with_mode`, `static double x = fabs(2.5);`
# compiled to a folded 2.5 even though the translation unit withdrew the
# hosted alias — GCC 16.2 refuses it ("initializer element is not constant").
# The reserved __builtin_* spellings keep folding in every mode.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
td=$(mktemp -d "${TMPDIR:-/tmp}/lccc-fno-builtin-folds.XXXXXX")
trap 'rm -rf "$td"' EXIT

cat > "$td/static_fabs.c" <<'EOF'
double fabs(double);
static double hosted_alias_folds = fabs(2.5);
double read(void) { return hosted_alias_folds; }
EOF
cat > "$td/static_builtin_fabs.c" <<'EOF'
static double explicit_spelling_folds = __builtin_fabs(2.5);
double read(void) { return explicit_spelling_folds; }
EOF

# Baseline (hosted default): the bare alias folds — the initializer is
# accepted and no libm call is emitted.
"$CCC" -O0 -c "$td/static_fabs.c" -o "$td/base.o"
if nm "$td/base.o" 2>/dev/null | grep -c ' U fabs' >/dev/null; then
    echo "FAIL: default-hosted static fabs(2.5) did not fold to a constant" >&2
    exit 1
fi

# The defect: -fno-builtin must make the bare spelling a non-constant
# initializer (the call stays a real call), i.e. the compile must FAIL.
if "$CCC" -O0 -fno-builtin -c "$td/static_fabs.c" -o "$td/refused.o" 2>"$td/refused.err"; then
    echo "FAIL: -fno-builtin still folded bare fabs(2.5) into a static initializer" >&2
    exit 1
fi
grep -qiE 'not.{0,20}constant|constant.{0,20}(required|initializer)' "$td/refused.err" || {
    cat "$td/refused.err" >&2
    echo "FAIL: the -fno-builtin static-init refusal is not the constant-initializer diagnostic" >&2
    exit 1
}

# -ffreestanding implies the same withdrawal (GCC: freestanding implies
# -fno-builtin).
if "$CCC" -O2 -ffreestanding -c "$td/static_fabs.c" -o "$td/freestanding.o" 2>/dev/null; then
    echo "FAIL: -ffreestanding still folded bare fabs(2.5) into a static initializer" >&2
    exit 1
fi

# The reserved spelling keeps folding under every mode.
for mode in "-O0 -fno-builtin" "-O2 -fno-builtin" "-ffreestanding"; do
    # shellcheck disable=SC2086
    "$CCC" $mode -c "$td/static_builtin_fabs.c" -o "$td/explicit.o" || {
        echo "FAIL: __builtin_fabs must fold even under $mode" >&2
        exit 1
    }
done

# Runtime calls under -fno-builtin are RETAINED (no libm shortcut folding of
# the bare spelling through a different path).
cat > "$td/runtime.c" <<'EOF'
double fabs(double);
double opaque(double);
double call_site(double x) { return fabs(x); }
EOF
for opt in -O0 -O2; do
    "$CCC" $opt -fno-builtin -S "$td/runtime.c" -o "$td/runtime.s"
    fn=$(awk '/call_site:/,/\.size.*call_site/' "$td/runtime.s")
    # herestring, not `echo | grep -q`: under `set -o pipefail` an early-exit
    # grep can SIGPIPE the producer and turn a match into a false failure.
    grep -Eq '(call|jmp)[q]?[[:space:]]+fabs(@PLT)?' <<<"$fn" || {
        echo "FAIL: -fno-builtin $opt did not retain the fabs call" >&2
        exit 1
    }
done

echo "fno-builtin fold contract: PASS (bare spellings refuse to fold in static init, __builtin_* folds everywhere, calls retained)"
