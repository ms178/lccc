#!/usr/bin/env bash
# End-to-end contract for hosted/freestanding, builtin, and tree-vectorize flags.
# These options used to disappear into the permissive unknown -f flag path.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
td=$(mktemp -d "${TMPDIR:-/tmp}/lccc-driver-options.XXXXXX")
trap 'rm -rf "$td"' EXIT
SKIP_MATRIX=${SKIP_TRAPV_MATRIX:-0}
while [ $# -gt 0 ]; do
    case $1 in
        --skip-trapv-matrix) SKIP_MATRIX=1; shift ;;
        -h|--help) echo "usage: $0 [--skip-trapv-matrix]"; exit 0 ;;
        *) echo "$0: unknown argument: $1" >&2; exit 2 ;;
    esac
done

cat > "$td/plain.c" <<'EOF'
int f(void) { return 42; }
EOF
for flag in \
    -flto -flto=auto \
    -fsanitize=address -fsanitize=undefined \
    -fvisibility=bogus -ftrivial-auto-var-init=bogus -fzero-call-used-regs=bogus \
    -fopenmp -freg-struct-return \
    -fno-profile-arcs -fno-test-coverage -fno-branch-probabilities -fno-auto-profile \
    -finstrument-functions-exclude-file-list=generated.c \
    -finstrument-functions-exclude-function-list=helper \
    -fprofile-update=unknown -fdiagnostics-color=unknown \
    -fprofile-dir= -fprofile-generate= -fprofile-use= -fauto-profile= \
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
# A table miss (a spelling newer than this tree) is DIAGNOSED but tolerated:
# the build continues (a probe must not be lied into "supported", but a newer
# GNU's inert preference must not break a real build either); strict mode is
# what turns the tolerance into the refusal a capability probe needs.
"$CCC" -fdefinitely-not-a-flag -c "$td/plain.c" -o "$td/miss.o" 2>"$td/miss.err" && \
    grep -Fq -- "-fdefinitely-not-a-flag" "$td/miss.err" || {
    echo "FAIL: a table-miss -f spelling must compile with a naming diagnostic" >&2
    exit 1
}
if LCCC_STRICT_OPTIONS=1 "$CCC" -fdefinitely-not-a-flag -c "$td/plain.c" -o "$td/miss2.o" 2>/dev/null; then
    echo "FAIL: LCCC_STRICT_OPTIONS=1 must refuse a table-miss -f spelling" >&2
    exit 1
fi
for flag in -fno-stack-protector -fno-stack-clash-protection -fno-lto -fsanitize=none -fno-strict-aliasing -fno-trapv; do
    "$CCC" "$flag" -c "$td/plain.c" -o "$td/noop.o"
done

cat > "$td/pgo.c" <<'EOF'
int main(void) { return 0; }
EOF
for order in before after; do
    profile_dir="$td/pgo-$order"
    mkdir -p "$profile_dir"
    if [[ $order == before ]]; then
        pgo_flags=(-fprofile-dir="$profile_dir" -fprofile-generate)
    else
        pgo_flags=(-fprofile-generate -fprofile-dir="$profile_dir")
    fi
    "$CCC" "${pgo_flags[@]}" "$td/pgo.c" -o "$td/pgo-$order.bin"
    "$td/pgo-$order.bin"
    if [[ -z $(find "$profile_dir" -maxdepth 1 -name '*.profraw' -print -quit) ]]; then
        echo "FAIL: -fprofile-dir before/after -fprofile-generate wrote no profile in $profile_dir" >&2
        exit 1
    fi
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

# GCC 16.2's positive BMM option implies BW and F. A subsequent negative
# BMM option revokes those implied prerequisites when they have no other source.
"$CCC" -dM -E -mavx512bmm -x c /dev/null >"$td/bmm-on.macros"
grep -q '^#define __AVX512BMM__ 1$' "$td/bmm-on.macros"
grep -q '^#define __AVX512F__ 1$' "$td/bmm-on.macros"
grep -q '^#define __AVX512BW__ 1$' "$td/bmm-on.macros"
"$CCC" -dM -E -mavx512bmm -mno-avx512bmm -x c /dev/null >"$td/bmm-off.macros"
if grep -q '^#define __AVX512' "$td/bmm-off.macros"; then
    echo "FAIL: -mno-avx512bmm left BMM-only AVX-512 grants enabled" >&2
    exit 1
fi
"$CCC" -dM -E -mavx512f -mavx512bmm -mno-avx512bmm -x c /dev/null >"$td/bmm-off-keep-f.macros"
grep -q '^#define __AVX512F__ 1$' "$td/bmm-off-keep-f.macros"
if grep -Eq '^#define __AVX512(BW|BMM)__' "$td/bmm-off-keep-f.macros"; then
    echo "FAIL: BMM-only BW grant survived its revocation alongside independent AVX512F" >&2
    exit 1
fi
"$CCC" -dM -E -mavx512bw -mavx512bmm -mno-avx512bmm -x c /dev/null >"$td/bmm-off-keep-bw.macros"
grep -q '^#define __AVX512F__ 1$' "$td/bmm-off-keep-bw.macros"
grep -q '^#define __AVX512BW__ 1$' "$td/bmm-off-keep-bw.macros"
if grep -q '^#define __AVX512BMM__' "$td/bmm-off-keep-bw.macros"; then
    echo "FAIL: -mno-avx512bmm did not revoke BMM beside independent BW" >&2
    exit 1
fi
"$CCC" -dM -E -mavx512bmm -mno-avx512bmm -mavx512bmm -x c /dev/null >"$td/bmm-last-on.macros"
grep -q '^#define __AVX512BMM__ 1$' "$td/bmm-last-on.macros"

cat > "$td/hosted-override.c" <<'EOF'
#if __STDC_HOSTED__ != 0
#error "a user -D must override the predefined __STDC_HOSTED__ value"
#endif
int hosted_override;
EOF
"$CCC" -fhosted -D__STDC_HOSTED__=0 -fsyntax-only "$td/hosted-override.c"

# GCC 16.2 defaults to GNU C23: an empty parameter list is a prototype with
# zero parameters. The explicit GNU17 escape hatch retains the older
# unspecified-parameters meaning, for both direct and function-pointer calls.
cat > "$td/empty-parameter-list.c" <<'EOF'
static int no_args() { return 42; }
int probe(void) { return no_args(1); }
EOF
cat > "$td/empty-declaration.c" <<'EOF'
extern int no_args();
int probe(void) { return no_args(1); }
EOF
cat > "$td/empty-function-pointer.c" <<'EOF'
static int no_args() { return 42; }
int probe(void) { int (*fp)() = no_args; return fp(1); }
EOF
cat > "$td/empty-function-pointer-typedef.c" <<'EOF'
typedef int (*no_args_fn)();
int probe(no_args_fn fn) { return fn(1); }
EOF
for source in \
    "$td/empty-parameter-list.c" \
    "$td/empty-declaration.c" \
    "$td/empty-function-pointer.c" \
    "$td/empty-function-pointer-typedef.c"; do
    if "$CCC" -fsyntax-only "$source" 2>"$td/default-dialect.err"; then
        echo "FAIL: default GNU C23 accepted an extra argument to an empty-list function" >&2
        exit 1
    fi
done
"$CCC" -std=gnu17 -fsyntax-only "$td/empty-parameter-list.c"
"$CCC" -std=gnu17 -fsyntax-only "$td/empty-declaration.c"
"$CCC" -std=gnu17 -fsyntax-only "$td/empty-function-pointer.c"
"$CCC" -std=gnu17 -fsyntax-only "$td/empty-function-pointer-typedef.c"
cat > "$td/void-parameter-list.c" <<'EOF'
static int no_args(void) { return 42; }
int probe(void) { return no_args(1); }
EOF
if "$CCC" -std=gnu17 -fsyntax-only "$td/void-parameter-list.c" 2>"$td/void-parameter-list.err"; then
    echo "FAIL: GNU C17 failed to treat (void) as a zero-parameter prototype" >&2
    exit 1
fi

# The conditional operator must form a composite function-pointer type. A
# prototype arm refines an unprototyped arm even when it is the else operand.
cat > "$td/conditional-function-pointer-valid.c" <<'EOF'
extern int old_style();
extern int prototype(void);
int probe(int choose) { return (choose ? old_style : prototype)(); }
EOF
"$CCC" -std=gnu17 -fsyntax-only "$td/conditional-function-pointer-valid.c"
cat > "$td/conditional-function-pointer-extra-argument.c" <<'EOF'
extern int old_style();
extern int prototype(void);
int probe(int choose) { return (choose ? old_style : prototype)(1); }
EOF
if "$CCC" -std=gnu17 -fsyntax-only \
    "$td/conditional-function-pointer-extra-argument.c" \
    2>"$td/conditional-function-pointer.err"; then
    echo "FAIL: GNU C17 conditional composite discarded its zero-parameter prototype" >&2
    exit 1
fi

# The policy is wired to every automatic vectorizer, not merely accepted by
# the argument parser. This fixture is independently required to emit AVX2 at
# -O2 by check_vectorize_isa_gate.sh; with the flag it must remain scalar.
vec_source="$repo/tests/regression/vectorize_isa_gate.c"
"$CCC" -O2 -march=x86-64-v3 -S "$vec_source" -o "$td/vector-default.s"
"$CCC" -O2 -fno-tree-vectorize -S "$vec_source" -o "$td/vector-disabled.s"
if ! grep -Eq '\bymm[0-9]+\b' "$td/vector-default.s"; then
    echo "FAIL: the vectorizer control fixture did not emit its expected AVX2 baseline" >&2
    exit 1
fi
if grep -Eq '\b[xyz]mm[0-9]+\b' "$td/vector-disabled.s"; then
    echo "FAIL: -fno-tree-vectorize left automatic SIMD in the fixture" >&2
    exit 1
fi
"$CCC" -O2 -march=x86-64-v3 -fno-tree-vectorize -ftree-vectorize -S "$vec_source" -o "$td/vector-reenabled.s"
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

# A sibling tail jump is still an out-of-line call site: folding policy is
# about whether the library invocation survives, not which branch mnemonic
# transfers to it.
CALL_RE='(call|jmp)[q]?[[:space:]]+strlen(@PLT)?'
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
    grep -Eq "(call|jmp)[q]?[[:space:]]+$name(@PLT)?" "$td/math-purity.s" || {
        echo "FAIL: -fno-builtin lost ordinary $name call to math/purity optimization" >&2
        exit 1
    }
done
# The cheap -O0/-O1 tiers use a separate math-intrinsic fold entry point.
# Verify that it receives the same per-TU policy as the main simplifier.
for opt in -O0 -O1; do
    "$CCC" "$opt" -fno-builtin -S "$td/math-purity.c" -o "$td/math-purity-$opt.s"
    grep -Eq '(call|jmp)[q]?[[:space:]]+sqrt(@PLT)?' "$td/math-purity-$opt.s" || {
        echo "FAIL: $opt -fno-builtin folded ordinary sqrt through the low-tier intrinsic pass" >&2
        exit 1
    }
done

# Per-name `-fno-builtin-<fn>` IS supported, with GCC's stickiness: the
# per-name withdrawal survives a later blanket `-fbuiltin`.  Verify the
# withdrawal is real (ordinary strlen stays a call) and scoped (the explicit
# __builtin_strlen spelling still folds).
cat > "$td/pername.c" <<'CEOF'
unsigned long per_name_withdrawn(void) {
    char buffer[strlen("abcdefghijkl")];
    return sizeof(buffer);
}
unsigned long per_name_explicit_still_folds(void) {
    char buffer[__builtin_strlen("abcdefghijkl")];
    return sizeof(buffer);
}
CEOF
"$CCC" -O2 -fno-builtin-strlen -fbuiltin -S "$td/pername.c" -o "$td/pername.s" || {
    echo "FAIL: -fno-builtin-strlen (per-name withdrawal) was refused; GCC keeps it sticky after -fbuiltin" >&2
    exit 1
}
withdrawn_fn=$(awk '/per_name_withdrawn:/,/\.size.*per_name_withdrawn/' "$td/pername.s")
# herestring, not `echo | grep -q`: under `set -o pipefail` an early-exit grep
# can SIGPIPE the producer and turn a match into a false failure.
grep -Eq '(call|jmp)[q]?[[:space:]]+strlen(@PLT)?' <<<"$withdrawn_fn" || {
    echo "FAIL: per-name -fno-builtin-strlen did not keep the ordinary strlen call" >&2
    exit 1
}
explicit_fn=$(awk '/per_name_explicit_still_folds:/,/\.size.*per_name_explicit_still_folds/' "$td/pername.s")
if grep -Eq '(call|jmp)[q]?[[:space:]]+strlen(@PLT)?' <<<"$explicit_fn"; then
    echo "FAIL: explicit __builtin_strlen must still fold under per-name withdrawal" >&2
    exit 1
fi

# Ignoring these yields correct (if differently-shaped or more conservative)
# code, so they are inert no-ops rather than refusals: a refusal would break
# the kernel/glibc builds that pass them unconditionally.
for flag in -fno-plt -fplt -fvisibility=hidden -fvisibility=default -ftrapv -fno-trapv \
    -ftrivial-auto-var-init=zero -ftrivial-auto-var-init=pattern \
    -fstack-clash-protection -fstack-protector-strong \
    -fzero-call-used-regs=used -fwhole-program -fno-whole-program \
    -fsemantic-interposition -fno-semantic-interposition; do
    "$CCC" -O2 "$flag" -S "$td/plain.c" -o "$td/noop.s" 2>/dev/null || {
        echo "FAIL: $flag must be accepted" >&2
        exit 1
    }
done

# Verify -fvisibility=hidden emits .hidden on defined non-static symbols while
# leaving extern symbols untouched.
cat > "$td/vis.c" <<'EOF'
extern int ext_fn(void);
int def_fn(void) { return ext_fn(); }
__attribute__((visibility("default"))) int pub_fn(void) { return 1; }
EOF
"$CCC" -O2 -fPIC -fvisibility=hidden -S "$td/vis.c" -o "$td/vis.s"
grep -Eq '^\s*\.hidden\s+def_fn\b' "$td/vis.s" || {
    echo "FAIL: -fvisibility=hidden did not emit .hidden def_fn" >&2
    exit 1
}
if grep -Eq '^\s*\.hidden\s+(ext_fn|pub_fn)\b' "$td/vis.s"; then
    echo "FAIL: -fvisibility=hidden marked extern or default-visibility symbol .hidden" >&2
    exit 1
fi

# Verify -fno-plt emits GOT-indirect call (*ext_fn@GOTPCREL(%rip)) and -fplt restores @PLT.
"$CCC" -O2 -fPIC -fno-plt -S "$td/vis.c" -o "$td/noplt.s"
grep -Eq '(call|jmp)[q]?[[:space:]]+\*ext_fn@GOTPCREL\(%rip\)' "$td/noplt.s" || {
    echo "FAIL: -fno-plt did not emit GOT-indirect call to ext_fn" >&2
    exit 1
}
"$CCC" -O2 -fPIC -fno-plt -fplt -S "$td/vis.c" -o "$td/plt-restored.s"
grep -Eq '(call|jmp)[q]?[[:space:]]+ext_fn@PLT' "$td/plt-restored.s" || {
    echo "FAIL: -fplt after -fno-plt did not restore @PLT call" >&2
    exit 1
}

# Verify -ftrapv aborts at runtime on signed overflow and -ftrivial-auto-var-init=pattern
# initializes uninitialized automatic storage while honoring __attribute__((uninitialized)).
cat > "$td/trapv_check.c" <<'EOF'
#include <limits.h>
__attribute__((noinline)) int ovf_add(int a, int b) { return a + b; }
int main(void) { return ovf_add(INT_MAX, 1); }
EOF
"$CCC" -O2 -ftrapv "$td/trapv_check.c" -o "$td/trapv_check"
set +e
"$td/trapv_check" >/dev/null 2>&1
trap_rc=$?
set -e
if [ "$trap_rc" -ne 134 ]; then
    echo "FAIL: -ftrapv signed overflow exited with $trap_rc (expected 134 SIGABRT)" >&2
    exit 1
fi

# The 6,776-case boundary matrix is split into one script per ABI so hosted CI
# can place -m64 and -m32 on different runners. Locally both legs run in
# parallel; --skip-trapv-matrix is the hosted core-only half of the contract.
if [[ $SKIP_MATRIX != 1 ]]; then
    pids=()
    for bits in -m64 -m32; do
        bash "$repo/tests/regression/check_trapv_boundary_matrix.sh" --bits "$bits" \
            > "$td/trapv-$bits.log" 2>&1 &
        pids+=("$!")
    done
    matrix_rc=0
    for pid in "${pids[@]}"; do
        wait "$pid" || matrix_rc=1
    done
    for bits in -m64 -m32; do
        cat "$td/trapv-$bits.log"
    done
    if [ "$matrix_rc" -ne 0 ]; then
        echo "FAIL: -ftrapv boundary matrix failed on one or both ABIs" >&2
        exit 1
    fi
fi

cat > "$td/autovar_check.c" <<'EOF'
#include <stdint.h>
__attribute__((noinline)) uint64_t read_pat(void) {
    uint64_t v;
    __asm__ __volatile__("" : "+r"(v));
    return v;
}
int main(void) { return read_pat() == 0xFEFEFEFEFEFEFEFEULL ? 0 : 1; }
EOF
"$CCC" -O2 -ftrivial-auto-var-init=pattern "$td/autovar_check.c" -o "$td/autovar_check"
"$td/autovar_check" || {
    echo "FAIL: -ftrivial-auto-var-init=pattern did not fill automatic variable with 0xFE" >&2
    exit 1
}

cat > "$td/clash_check.c" <<'EOF'
__attribute__((noinline)) int touch_deep_frame(int idx) {
    volatile char buf[16384];
    buf[0] = 1;
    buf[idx] = 2;
    return buf[0] + buf[idx];
}
int main(void) { return touch_deep_frame(12000) == 3 ? 0 : 1; }
EOF
"$CCC" -O2 -fstack-clash-protection -S "$td/clash_check.c" -o "$td/clash_on.s"
"$CCC" -O2 -fno-stack-clash-protection -S "$td/clash_check.c" -o "$td/clash_off.s"
if cmp -s "$td/clash_on.s" "$td/clash_off.s"; then
    echo "FAIL: -fstack-clash-protection emitted identical assembly to -fno-stack-clash-protection on a 16 KiB frame" >&2
    exit 1
fi
"$CCC" -O2 -fstack-clash-protection "$td/clash_check.c" -o "$td/clash_check"
"$td/clash_check" || {
    echo "FAIL: -fstack-clash-protection binary failed at runtime" >&2
    exit 1
}

echo "driver option contract: PASS (hosted/freestanding macros, builtin gating, accepted/refused vectorizer and builtin flags, -ftrapv m64/m32 overflow matrix, stack clash probes)"
