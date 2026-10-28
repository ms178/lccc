#!/usr/bin/env bash
# GP-24: inline-asm operand and template validation must match GCC.
#
# WHY: the x86 back ends classify an operand constraint with a `match` whose
# catch-all arm is `_ => {}`, so before this check an unmodelled constraint
# letter ("z", "j", "f", ...) was silently reclassified as a general-purpose
# register and the statement was emitted with the wrong register class — a
# miscompile rather than a rejected program. The parser now diagnoses the
# three classes GCC diagnoses, with GCC's wording, before any code is generated.
#
# Every expectation below was measured against gcc 16.2 on x86-64:
#   "=r" on an input      -> error: input operand constraint contains '='
#   "+r" on an input      -> error: input operand constraint contains '+'
#   unmodelled letter     -> error: impossible constraint in 'asm'
#   "%7" with 2 operands  -> error: invalid 'asm': operand number out of range
#   "I"(300) / "I"(x)     -> warning: 'asm' operand N probably does not match
#                            constraints, then impossible constraint in 'asm'
# ... and, just as important, the shapes that must stay accepted: multi-
# alternative constraints ("Ir" with a variable, the kernel's `btsq`), tied
# operands, explicit register operands, x87 "t"/"u", "Yz", and every %-code the
# substitution pass rewrites.
set -uo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC:-gcc-16}
TMP=${TMPDIR:-/tmp}/lccc-asmdiag.$$
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP"
fail=0

# run_case <name> <expect: error|clean> <body>
run_case() {
    local name=$1 expect=$2 body=$3
    printf '%s\n' "$body" >"$TMP/t.c"
    local out
    out=$("$CCC" -fsyntax-only "$TMP/t.c" 2>&1)
    if [[ $expect == error ]]; then
        if [[ $out != *"$EXPECT_MESSAGE"* ]]; then
            echo "FAIL $name: expected '$EXPECT_MESSAGE', got:" >&2
            echo "$out" >&2
            fail=1
        fi
    else
        if [[ -n $out ]]; then
            echo "FAIL $name: expected no diagnostic, got:" >&2
            echo "$out" >&2
            fail=1
        fi
    fi
}

EXPECT_MESSAGE="input operand constraint contains '='"
run_case eq_on_input error \
    'int f(int x){int r;__asm__ volatile("movl %1, %0":"=r"(r):"=r"(x));return r;}'

EXPECT_MESSAGE="input operand constraint contains '+'"
run_case plus_on_input error \
    'int f(int x){int r;__asm__ volatile("movl %1, %0":"=r"(r):"+r"(x));return r;}'

EXPECT_MESSAGE="impossible constraint in 'asm'"
run_case unmodelled_letter error \
    'int f(int x){int r;__asm__ volatile("movl %1, %0":"=r"(r):"z"(x));return r;}'
# `f` is GCC's any-x87-register class and this compiler now classifies it as
# st(0), so the validator must accept it (it used to fall through to GpReg).
run_case x87_register_class clean \
    'int f(double x){double r;__asm__ volatile("fchs":"=f"(r):"0"(x));return (int)r;}'
run_case unmodelled_letter2 error \
    'int f(int x){int r;__asm__ volatile("movl %1, %0":"=r"(r):"P"(x));return r;}'
run_case immediate_on_variable error \
    'int f(int x){int r;__asm__ volatile("movl %1, %0":"=r"(r):"I"(x));return r;}'

EXPECT_MESSAGE="probably does not match constraints"
run_case immediate_out_of_range error \
    'int f(void){int r;__asm__ volatile("movl %1, %0":"=r"(r):"I"(300));return r;}'

EXPECT_MESSAGE="invalid 'asm': operand number out of range"
run_case template_index error \
    'int f(int x){int r;__asm__ volatile("movl %7, %0":"=r"(r):"r"(x));return r;}'
run_case template_modifier_index error \
    'int f(int x){int r;__asm__ volatile("movb %b2, %b0":"=r"(r):"r"(x));return r;}'

# Shapes that must stay accepted: GCC compiles each of these, so refusing one
# would be a compatibility regression.
for body in \
    'int f(int x){int r;__asm__ volatile("movl %1, %0":"=r"(r):"r"(x));return r;}' \
    'int f(unsigned long *a,long n){int c;__asm__ volatile("btsq %2, %1":"=@ccc"(c),"+m"(*a):"Ir"(n));return c;}' \
    'int f(int x){int r;__asm__ volatile("addl %1, %0":"=r"(r):"0"(x));return r;}' \
    'int f(int x){int r;__asm__ volatile("addl %1, %0":"+r"(r):"g"(x));return r;}' \
    'int f(void){int r=0;__asm__ volatile("movl $7, %0":"=r"(r):);return r;}' \
    'int f(void){int r;__asm__ volatile("movl %1, %0":"=r"(r):"i"(7));return r;}' \
    'int f(void){int r;__asm__ volatile("movl $1, %0":"=r"(r));return r;}' \
    'int f(char x){int r;__asm__ volatile("movsbl %b1, %0":"=r"(r):"q"(x));return r;}' \
    'int f(int x){int r;__asm__ volatile("movl %k1, %0":"=r"(r):"r"(x));return r;}' \
    'int f(short x){int r;__asm__ volatile("movswl %w1, %0":"=r"(r):"r"(x));return r;}' \
    'int f(double x){double r;__asm__ volatile("movsd %1, %0":"=x"(r):"x"(x));return (int)r;}' \
    'int f(void){int r;__asm__ volatile("xorl %0, %0":"=r"(r)::"cc");return r;}'
do
    expect=$("$GCC" -c -x c -o /dev/null - <<<"$body" 2>&1)
    if [[ -n $expect ]]; then
        echo "skip: gcc-16 rejects a case this check treats as valid: $body" >&2
        echo "$expect" >&2
        fail=1
        continue
    fi
    run_case "accepted:${body:0:28}" clean "$body"
done

if ((fail)); then
    echo "check_asm_operand_diagnostics: FAILED" >&2
    exit 1
fi
echo "check_asm_operand_diagnostics: PASSED"
