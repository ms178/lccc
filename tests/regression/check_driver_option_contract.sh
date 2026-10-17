#!/usr/bin/env bash
# End-to-end contract for hosted/freestanding, builtin, and tree-vectorize flags.
# These options used to disappear into the permissive unknown -f flag path.
set -euo pipefail

CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
td=$(mktemp -d "${TMPDIR:-/tmp}/lccc-driver-options.XXXXXX")
trap 'rm -rf "$td"' EXIT

cat > "$td/plain.c" <<'EOF'
int f(void) { return 42; }
EOF
for flag in -fdefinitely-not-a-flag -flto -flto=auto -fsanitize=address -fsanitize=undefined -fstack-clash-protection -ftrapv -fvisibility=hidden -fno-plt -fopenmp -freg-struct-return; do
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
"$CCC" -fno-tree-vectorize -fsyntax-only "$td/hosted.c"
"$CCC" -ftree-vectorize -fsyntax-only "$td/hosted.c"

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
