#!/usr/bin/env bash
# Gate: a vector return must not cost a stack round trip, and every lane must
# survive it.
#
# Two independent claims, because they fail independently:
#
#   1. VALUES.  Run a program that returns __m128/__m256 values from functions
#      and forwards them through a second call.  Any lane lost in the
#      `%xmm0`-to-GP-half decomposition shows up here.
#   2. COST.  Count the emitted instructions of
#           v8 f(v8 a){ return ext8(a); }
#      GCC 16.2 emits `jmp ext8` (1 instruction).  LCCC currently emits 25 --
#      four aligned stack slots and a GP-half rebuild -- because the IR carries
#      a vector return as packed I64 (src/ir/lowering/lower.rs, "A small vector
#      travels as packed I64 data").  The budget below is the boundary: it fails
#      on the current tree and starts passing when the register home is kept, so
#      it cannot be satisfied accidentally by an unrelated change.
#
# Usage: check_vector_return_abi.sh [--lccc PATH] [--reference CC] [--budget N]
set -euo pipefail

LCCC=""; REFERENCE=""; BUDGET=6
while [ $# -gt 0 ]; do
    case "$1" in
        --lccc) LCCC="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        --budget) BUDGET="$2"; shift 2 ;;
        -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "check_vector_return_abi: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
CC="${LCCC:-$ROOT/target/fastbuild/lccc}"
DIR="$ROOT/tests/regression/vector_abi"
[ -x "$CC" ] || { echo "FAIL: lccc binary '$CC' is not executable" >&2; exit 1; }

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
echo "check_vector_return_abi: vector returns"

# --- 1. values -------------------------------------------------------------
for flags in "-msse2" "-mavx2"; do
    $CC -O2 -w $flags "$DIR/vector_return_abi.c" "$DIR/check_vector_return_abi.c" \
        -o "$WORK/t" 2>"$WORK/cc.log" \
        || { echo "FAIL: cannot build the value probe ($flags)" >&2; cat "$WORK/cc.log" >&2; exit 1; }
    if "$WORK/t" > "$WORK/out.txt" 2>&1; then
        echo "  ok  $flags: $(cat "$WORK/out.txt")"
    else
        echo "FAIL: $flags vector return corrupted lanes:" >&2
        cat "$WORK/out.txt" >&2
        exit 1
    fi
done

# --- 2. cost ---------------------------------------------------------------
cat > "$WORK/cost.c" <<'EOF'
typedef float v8 __attribute__((vector_size(32)));
extern v8 ext8(v8);
v8 f(v8 a){ return ext8(a); }
EOF
$CC -O2 -w -mavx2 -S "$WORK/cost.c" -o "$WORK/cost.s" 2>/dev/null \
    || { echo "FAIL: cannot emit assembly for the cost probe" >&2; exit 1; }
# Count the body of `f:` only, between its label and `ret`.
body=$(awk '/^f:/{on=1;next} on&&/^[[:space:]]*ret/{exit} on' "$WORK/cost.s")
n=$(printf '%s\n' "$body" | grep -cE '^[[:space:]]+[a-z]' || true)
if [ "$n" -gt "$BUDGET" ]; then
    echo "FAIL: v8 f(v8) costs $n instructions (budget $BUDGET; GCC 16.2 emits 1)." >&2
    echo "      The vector return is being rebuilt through GP halves or a stack" >&2
    echo "      slot instead of staying in %ymm0.  Emitted body:" >&2
    printf '%s\n' "$body" | sed 's/^/        /' >&2
    exit 1
fi
echo "  ok  v8 f(v8) costs $n instruction(s) (budget $BUDGET)"

if [ -n "$REFERENCE" ] && command -v "$REFERENCE" >/dev/null 2>&1; then
    $REFERENCE -O2 -w -mavx2 -S "$WORK/cost.c" -o "$WORK/ref.s" 2>/dev/null || true
    if [ -s "$WORK/ref.s" ]; then
        rb=$(awk '/^f:/{on=1;next} on&&/^[[:space:]]*ret/{exit} on' "$WORK/ref.s")
        rn=$(printf '%s\n' "$rb" | grep -cE '^[[:space:]]+[a-z]' || true)
        rn=$((rn + 1))   # `jmp ext8` is the whole body and ends the function
        echo "  ref $REFERENCE: $rn instruction(s)"
    fi
fi

echo "PASS: vector returns preserve every lane within the instruction budget"
