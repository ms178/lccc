#!/usr/bin/env bash
# check_arm_inline_asm_provenance.sh — user-authored AArch64 assembly is
# EXEMPT from the -mgeneral-regs-only SIMD guard, compiler-generated SIMD is
# NOT, and the exemption cannot leak (PR #821 F1, review of 2026-10-14).
#
# THE GAP THIS PINS
# -----------------
# The x86/i686 emitters bracket inline-asm bodies with `#APP`/`#NO_APP`, but
# the AArch64 emitter inherited the hook's None default: user SIMD assembly
# reached the driver's final-text scanner unmarked, and
#
#     void user_simd(void) { __asm__ volatile("movi v0.16b, #0" ::: "v0"); }
#
# was REJECTED under -mgeneral-regs-only ("FP/SIMD register use is forbidden")
# — the guard designed for compiler-generated code ate the user's own
# instruction.  The pre-fix unit tests only fed hand-built marker strings to
# the scanner, so the missing producer was invisible to them.
#
# THE CONTRACT (all rows compile REAL C through lccc-arm, no hand-built asm)
#   1. ordinary-function inline SIMD asm      -> compiles
#   2. compiler-generated FP (fmov s0)        -> rejected, located
#   3. mixed function: user asm exempt, the   -> rejected (no leak)
#      compiler's own s0 handling still trips
#   4. naked-function raw asm (whole-body     -> compiles
#      user text)
#   5. top-level file-scope asm               -> compiles
#   6. provenance markers reach the .s for    -> #APP/#NO_APP present
#      all three user-asm forms
#   7. default-mode .s differs ONLY by those  -> diff clean
#      comment lines; the marked .s survives  -> lccc-arm -c (GNU as) exits 0
#      the real assembler
#
# Mutation check (--self-test): a fake compiler that (a) rejects user SIMD
# under the flag (the pre-fix behavior) must fail row 1, and (b) emits no
# markers must fail row 6.
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SELF=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
LCCC=${CCC_ARM:-$ROOT/target/fastbuild/lccc-arm}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

fail=0
step() {
    local name=$1 expr=$2
    if eval "$expr"; then
        printf 'PASS %s\n' "$name"
    else
        printf 'FAIL %s\n' "$name"
        fail=1
    fi
}

if [[ ${1:-} == --self-test ]]; then
    # Fake A: the pre-fix compiler — rejects user SIMD under the flag.
    cat > fake-rejecting-lccc <<'EOF'
#!/usr/bin/env bash
for a in "$@"; do
    case "$a" in
        -mgeneral-regs-only) flag=1;;
        *.c) src=$a;;
    esac
done
if [[ ${flag:-0} == 1 ]] && grep -q 'v0\|movi' "$src" 2>/dev/null; then
    echo "fake: FP/SIMD register use is forbidden by -mgeneral-regs-only" >&2
    exit 1
fi
# behave like a compiler otherwise: emit asm with no markers
out=""; sflag=0; ofile=""
prev=""
for a in "$@"; do
    case "$prev" in
        -o) ofile=$a;;
    esac
    case "$a" in
        -S) sflag=1;;
        -o) ;;
    esac
    prev=$a
done
if [[ $sflag == 1 ]]; then
    printf '.text\n.globl f\nf:\n\tret\n' > "${ofile:-a.s}"
fi
exit 0
EOF
    chmod +x fake-rejecting-lccc
    out=$(CCC_ARM="$PWD/fake-rejecting-lccc" bash "$SELF" 2>&1)
    if grep -q '^FAIL user SIMD in ordinary function: compiles' <<<"$out"; then
        echo "PASS self-test A (rejecting fake is caught)"
    else
        echo "FAIL self-test A: the fake pre-fix compiler was NOT rejected"
        fail=1
    fi
    # Fake B: accepts everything but emits no markers.
    cat > fake-unmarked-lccc <<'EOF'
#!/usr/bin/env bash
sflag=0; ofile=""
prev=""
for a in "$@"; do
    case "$prev" in -o) ofile=$a;; esac
    case "$a" in -S) sflag=1;; esac
    prev=$a
done
if [[ $sflag == 1 ]]; then printf '.text\nf:\n\tret\n' > "${ofile:-a.s}"; fi
exit 0
EOF
    chmod +x fake-unmarked-lccc
    out=$(CCC_ARM="$PWD/fake-unmarked-lccc" bash "$SELF" 2>&1)
    if grep -q '^FAIL markers: r1' <<<"$out"; then
        echo "PASS self-test B (unmarked fake is caught)"
    else
        echo "FAIL self-test B: the marker-less fake was NOT rejected"
        fail=1
    fi
    exit $fail
fi

[[ -x "$LCCC" ]] || { echo "FAIL: lccc-arm not built at $LCCC (cargo build --profile fastbuild)" >&2; exit 1; }

# --- Row 1: ordinary-function inline SIMD asm compiles under the flag --------
printf 'void user_simd(void) {\n    __asm__ volatile("movi v0.16b, #0" ::: "v0");\n}\n' > r1.c
"$LCCC" -O2 -mgeneral-regs-only -S -o r1.s r1.c 2>err; rc=$?
step "user SIMD in ordinary function: compiles" '(( rc == 0 ))'

# --- Row 2: compiler-generated FP is still rejected, with a location ---------
printf 'float fp(float a, float b) { return a * b; }\n' > r2.c
"$LCCC" -O2 -mgeneral-regs-only -S -o r2.s r2.c 2>err; rc=$?
step "compiler-generated FP: rejected"          '(( rc != 0 ))'
step "compiler-generated FP: located diagnostic" 'grep -q "r2.c: in function .fp." err'

# --- Row 3: mixed function — the exemption does not leak ---------------------
# The call to g() forces the compiler to spill/restore the FP arguments
# through the SIMD register file (fmov ... s0), which stays outside any
# marker bracket and must still be flagged.
printf 'float g(float a, float b);\nvoid mixed(float a, float b) {\n    __asm__ volatile("movi v0.16b, #0" ::: "v0");\n    g(a, b);\n}\n' > r3.c
"$LCCC" -O2 -mgeneral-regs-only -S -o r3.s r3.c 2>err; rc=$?
step "mixed function: compiler FP still flagged" '(( rc != 0 ))'
step "mixed function: offending line is not user asm" 'grep -q "offending instruction" err && ! grep -q "movi" err'

# --- Row 4: naked-function raw asm compiles under the flag -------------------
printf '__attribute__((naked)) void nk(void) {\n    __asm__ volatile("movi v0.16b, #0\\n\\tret");\n}\n' > r4.c
"$LCCC" -O2 -mgeneral-regs-only -S -o r4.s r4.c 2>err; rc=$?
step "naked-function raw asm: compiles" '(( rc == 0 ))'

# --- Row 5: top-level file-scope asm compiles under the flag -----------------
printf '__asm__("movi v0.16b, #0\\n");\nvoid f(void) {}\n' > r5.c
"$LCCC" -O2 -mgeneral-regs-only -S -o r5.s r5.c 2>err; rc=$?
step "top-level asm: compiles" '(( rc == 0 ))'

# --- Row 6: provenance markers reach the .s for all three forms --------------
for f in r1 r4 r5; do
    step "markers: $f" 'grep -q "^#APP" '"$f"'.s && grep -q "^#NO_APP" '"$f"'.s'
done

# --- Row 7: default mode differs ONLY by the marker comment lines; the ------
# --- marked .s assembles through the real assembler --------------------------
"$LCCC" -O2 -S -o r1_default.s r1.c 2>/dev/null
diff <(grep -vE '^#(NO_)?APP$' r1.s) r1_default.s >/dev/null
step "default mode: identical except markers" '(( $? == 0 ))'
"$LCCC" -O2 -c -o r1.o r1.c 2>err; rc=$?
step "marked .s survives the real assembler" '(( rc == 0 ))'

if (( fail )); then
    echo "FAIL: AArch64 inline-asm provenance contract violated"
    exit 1
fi
echo "PASS: user AArch64 asm exempt (ordinary/naked/top-level), compiler SIMD still rejected, no leak, markers assembler-safe"
