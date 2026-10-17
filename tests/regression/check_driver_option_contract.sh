#!/usr/bin/env bash
# End-to-end contract for hosted/freestanding, builtin, and tree-vectorize flags.
# These options used to disappear into the permissive unknown -f flag path.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
td=$(mktemp -d "${TMPDIR:-/tmp}/lccc-driver-options.XXXXXX")
trap 'rm -rf "$td"' EXIT

cat > "$td/plain.c" <<'EOF'
int f(void) { return 42; }
EOF
for flag in \
    -fdefinitely-not-a-flag \
    -flto -flto=auto \
    -fsanitize=address -fsanitize=undefined \
    -fstack-clash-protection -ftrapv -fvisibility=hidden -fno-plt \
    -fsemantic-interposition -fno-semantic-interposition \
    -fopenmp -freg-struct-return \
    -fno-profile-arcs -fno-test-coverage -fno-branch-probabilities -fno-auto-profile \
    -finstrument-functions-exclude-file-list=generated.c \
    -finstrument-functions-exclude-function-list=helper \
    -fprofile-update=unknown -fdiagnostics-color=unknown \
    -fpatchable-function-entry=bogus -fpatchable-function-entry=2,0,1 \
    -fpatchable-function-entry=1,2; do
    for order in before after; do
        args=("$flag" -v)
        [[ $order == before ]] && args=(-v "$flag")
        if "$CCC" "${args[@]}" -c "$td/plain.c" -o "$td/refused.o" >"$td/out" 2>"$td/err"; then
            echo "FAIL: $flag silently accepted ($order -v)" >&2
            exit 1
        fi
        grep -Fq -- "$flag" "$td/err" || { cat "$td/err" >&2; exit 1; }
    done
done
for flag in -fno-stack-protector -fno-stack-clash-protection -fno-lto -fsanitize=none -fno-strict-aliasing -fno-trapv; do
    "$CCC" "$flag" -c "$td/plain.c" -o "$td/noop.o"
done

cat > "$td/hosted.c" <<'EOF'
#if __STDC_HOSTED__ != 1
#error "-fhosted must define __STDC_HOSTED__ as 1"
#endif
int hosted_mode;
EOF
cat > "$td/freestanding.c" <<'EOF'
#if __STDC_HOSTED__ != 0
#error "-ffreestanding must define __STDC_HOSTED__ as 0"
#endif
int freestanding_mode;
EOF
"$CCC" -fhosted -fsyntax-only "$td/hosted.c"
"$CCC" -ffreestanding -fsyntax-only "$td/freestanding.c"
"$CCC" -fno-builtin -fsyntax-only "$td/hosted.c"
cat > "$td/hosted-override.c" <<'EOF'
#if __STDC_HOSTED__ != 0
#error "a user -D must override the predefined __STDC_HOSTED__ value"
#endif
int hosted_override;
EOF
"$CCC" -fhosted -D__STDC_HOSTED__=0 -fsyntax-only "$td/hosted-override.c"

# The policy is wired to every automatic vectorizer, not merely accepted by
# the argument parser. This fixture is independently required to emit AVX2 at
# -O2 by check_vectorize_isa_gate.sh; with the flag it must remain scalar.
vec_source="$repo/tests/regression/vectorize_isa_gate.c"
"$CCC" -O2 -S "$vec_source" -o "$td/vector-default.s"
"$CCC" -O2 -fno-tree-vectorize -S "$vec_source" -o "$td/vector-disabled.s"
if ! grep -Eq '\bymm[0-9]+\b' "$td/vector-default.s"; then
    echo "FAIL: the vectorizer control fixture did not emit its expected AVX2 baseline" >&2
    exit 1
fi
if grep -Eq '\b[xyz]mm[0-9]+\b' "$td/vector-disabled.s"; then
    echo "FAIL: -fno-tree-vectorize left automatic SIMD in the fixture" >&2
    exit 1
fi
"$CCC" -O2 -fno-tree-vectorize -ftree-vectorize -S "$vec_source" -o "$td/vector-reenabled.s"
if ! grep -Eq '\bymm[0-9]+\b' "$td/vector-reenabled.s"; then
    echo "FAIL: a later -ftree-vectorize did not re-enable automatic vectorization" >&2
    exit 1
fi
"$CCC" -O2 -ftree-vectorize -fno-tree-vectorize -S "$vec_source" -o "$td/vector-disabled-last.s"
if grep -Eq '\b[xyz]mm[0-9]+\b' "$td/vector-disabled-last.s"; then
    echo "FAIL: the last -fno-tree-vectorize did not disable automatic vectorization" >&2
    exit 1
fi
"$CCC" -O2 -fno-tree-vectorize "$vec_source" -o "$td/vector-disabled"
if [[ $("$td/vector-disabled") != "fail=0" ]]; then
    echo "FAIL: -fno-tree-vectorize changed program semantics" >&2
    exit 1
fi

cat > "$td/strlen-vla.c" <<'EOF'
extern unsigned long strlen(const char *);
unsigned long ordinary_strlen_vla(void) {
    char buffer[strlen("abcdefghijkl")];
    return sizeof(buffer);
}
unsigned long explicit_builtin_strlen_vla(void) {
    char buffer[__builtin_strlen("abcdefghijkl")];
    return sizeof(buffer);
}
EOF
"$CCC" -O2 -S "$td/strlen-vla.c" -o "$td/hosted.s"
"$CCC" -O2 -fno-builtin -S "$td/strlen-vla.c" -o "$td/no-builtin.s"

CALL_RE='call[q]?[[:space:]]+strlen(@PLT)?'
if grep -Eq "$CALL_RE" "$td/hosted.s"; then
    echo "FAIL: hosted -O2 unexpectedly retained strlen call" >&2
    grep -nE "$CALL_RE" "$td/hosted.s" >&2
    exit 1
fi
calls=$(grep -Ec "$CALL_RE" "$td/no-builtin.s" || true)
if [[ $calls != 1 ]]; then
    echo "FAIL: -O2 -fno-builtin should retain only the ordinary strlen call; found $calls" >&2
    grep -nE 'strlen|call' "$td/no-builtin.s" >&2 || true
    exit 1
fi
# Verify that the parsed option order reaches the actual folds, not just the
# Driver fields. Explicit builtins continue to fold in either mode.
"$CCC" -O2 -fno-builtin -fbuiltin -S "$td/strlen-vla.c" -o "$td/builtin-last.s"
if grep -Eq "$CALL_RE" "$td/builtin-last.s"; then
    echo "FAIL: a later -fbuiltin did not re-enable hosted strlen folding" >&2
    exit 1
fi
"$CCC" -O2 -fbuiltin -fno-builtin -S "$td/strlen-vla.c" -o "$td/no-builtin-last.s"
calls=$(grep -Ec "$CALL_RE" "$td/no-builtin-last.s" || true)
if [[ $calls != 1 ]]; then
    echo "FAIL: a later -fno-builtin did not preserve the ordinary strlen call (found $calls)" >&2
    exit 1
fi
"$CCC" -O2 -ffreestanding -S "$td/strlen-vla.c" -o "$td/freestanding.s"
calls=$(grep -Ec "$CALL_RE" "$td/freestanding.s" || true)
if [[ $calls != 1 ]]; then
    echo "FAIL: -ffreestanding did not disable the ordinary strlen fold (found $calls)" >&2
    exit 1
fi
"$CCC" -O2 -ffreestanding -fhosted -S "$td/strlen-vla.c" -o "$td/hosted-last.s"
if grep -Eq "$CALL_RE" "$td/hosted-last.s"; then
    echo "FAIL: a later -fhosted did not restore hosted strlen folding" >&2
    exit 1
fi

# strlen observes the first NUL, not the literal's storage extent.
cat > "$td/embedded-nul.c" <<'EOF'
extern unsigned long strlen(const char *);
unsigned long embedded(void) { return strlen("ab\0cd"); }
int main(void) { return embedded() != 2 || __builtin_strlen("ab\0cd") != 2; }
EOF
"$CCC" -O2 "$td/embedded-nul.c" -o "$td/embedded-nul"
"$td/embedded-nul"

cat > "$td/math-purity.c" <<'EOF'
extern double sqrt(double);
extern unsigned long strlen(const char *);
double root(double x) { return sqrt(x); }
int observable(void) { strlen("abc"); return 0; }
EOF
"$CCC" -O2 -fno-builtin -S "$td/math-purity.c" -o "$td/math-purity.s"
for name in sqrt strlen; do
    grep -Eq "call[q]?[[:space:]]+$name(@PLT)?" "$td/math-purity.s" || {
        echo "FAIL: -fno-builtin lost ordinary $name call to math/purity optimization" >&2
        exit 1
    }
done
# The cheap -O0/-O1 tiers use a separate math-intrinsic fold entry point.
# Verify that it receives the same per-TU policy as the main simplifier.
for opt in -O0 -O1; do
    "$CCC" "$opt" -fno-builtin -S "$td/math-purity.c" -o "$td/math-purity-$opt.s"
    grep -Eq 'call[q]?[[:space:]]+sqrt(@PLT)?' "$td/math-purity-$opt.s" || {
        echo "FAIL: $opt -fno-builtin folded ordinary sqrt through the low-tier intrinsic pass" >&2
        exit 1
    }
done

if "$CCC" -fno-builtin-strlen -fsyntax-only "$td/hosted.c" >"$td/refused.out" 2>"$td/refused.err"; then
    echo "FAIL: unsupported per-function -fno-builtin-strlen was silently accepted" >&2
    exit 1
fi
if ! grep -q 'per-function builtin control is unsupported' "$td/refused.err"; then
    echo "FAIL: -fno-builtin-strlen failed without an explicit unsupported-option diagnostic" >&2
    cat "$td/refused.err" >&2
    exit 1
fi

echo "driver option contract: PASS (hosted/freestanding macros, builtin gating, accepted/refused vectorizer and builtin flags)"
