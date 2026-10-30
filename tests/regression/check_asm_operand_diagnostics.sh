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

# GCC defers immediate-only validation in an inline body until the caller's
# argument is substituted. Kernel helpers use this for static branches and
# BUG metadata. The constant call must compile; a dynamic call must be rejected
# after optimization rather than receiving the backend's zero placeholder.
cat >"$TMP/always-inline-immediate.c" <<'EOF'
static __attribute__((always_inline)) inline int imm(int x)
{
    int y;
    __asm__("movl %1, %0" : "=r"(y) : "i"(x));
    return y;
}
int constant_call(void) { return imm(7); }
EOF
if ! "$CCC" -O2 -c "$TMP/always-inline-immediate.c" -o "$TMP/always-inline-immediate.o"; then
    echo "FAIL always_inline_constant_immediate: constant call should compile" >&2
    fail=1
fi
cat >"$TMP/always-inline-dynamic.c" <<'EOF'
static __attribute__((always_inline)) inline int imm(int x)
{
    int y;
    __asm__("movl %1, %0" : "=r"(y) : "i"(x));
    return y;
}
int dynamic_call(int x) { return imm(x); }
EOF
if out=$("$CCC" -O2 -c "$TMP/always-inline-dynamic.c" -o "$TMP/always-inline-dynamic.o" 2>&1); then
    echo "FAIL always_inline_dynamic_immediate: dynamic value was accepted" >&2
    fail=1
elif [[ $out != *"non-constant operand for immediate-only inline asm constraint"* ]]; then
    echo "FAIL always_inline_dynamic_immediate: missing actionable diagnostic:" >&2
    echo "$out" >&2
    fail=1
fi

# Local scalar initializers are often folded to immediate asm operands only
# after optimization (Linux's __WARN_FLAGS creates a local __auto_type __flags).
# Keep that path distinct from inline-parameter substitution above.
cat >"$TMP/local-constant-immediate.c" <<'EOF'
void local_constant(void)
{
    unsigned int flags = 9;
    __asm__ volatile(".long %c0" : : "i"(flags));
}
EOF
if ! "$CCC" -O2 -c "$TMP/local-constant-immediate.c" -o "$TMP/local-constant-immediate.o"; then
    echo "FAIL local_constant_immediate: optimized local constant should compile" >&2
    fail=1
fi
cat >"$TMP/local-dynamic-immediate.c" <<'EOF'
void local_dynamic(unsigned int flags)
{
    __asm__ volatile(".long %c0" : : "i"(flags));
}
EOF
if out=$("$CCC" -O2 -c "$TMP/local-dynamic-immediate.c" -o "$TMP/local-dynamic-immediate.o" 2>&1); then
    echo "FAIL local_dynamic_immediate: dynamic value was accepted" >&2
    fail=1
elif [[ $out != *"non-constant operand for immediate-only inline asm constraint"* ]]; then
    echo "FAIL local_dynamic_immediate: missing actionable diagnostic:" >&2
    echo "$out" >&2
    fail=1
fi

EXPECT_MESSAGE="probably does not match constraints"
run_case immediate_out_of_range error \
    'int f(void){int r;__asm__ volatile("movl %1, %0":"=r"(r):"I"(300));return r;}'

EXPECT_MESSAGE="invalid 'asm': operand number out of range"
run_case template_index error \
    'int f(int x){int r;__asm__ volatile("movl %7, %0":"=r"(r):"r"(x));return r;}'
run_case template_modifier_index error \
    'int f(int x){int r;__asm__ volatile("movb %b2, %b0":"=r"(r):"r"(x));return r;}'

# Shapes that must stay accepted: GCC compiles each of these, so refusing one
# would be a compatibility regression. The last three are the ones an
# unmeasured letter table gets wrong: gcc 16.2 accepts `"U"` for a register and
# for an immediate, `"Ci"` for an integer constant (a `C`-prefixed pair, like
# `Yz`, is a class rather than a letter) and `"M"(3)` inside its range.
for body in \
    'int f(int x){int r;__asm__ volatile("movl %1, %0":"=r"(r):"r"(x));return r;}' \
    'int f(unsigned long *a,long n){int c;__asm__ volatile("btsq %2, %1":"=@ccc"(c),"+m"(*a):"Ir"(n));return c;}' \
    'int f(int x){int r;__asm__ volatile("addl %1, %0":"=r"(r):"0"(x));return r;}' \
    'int f(unsigned long x){unsigned long y;__asm__ volatile("" : [sum] "=r"(y) : "[sum]"(x));return y;}' \
    'int f(int x){int r;__asm__ volatile("addl %1, %0":"+r"(r):"g"(x));return r;}' \
    'int f(void){int r=0;__asm__ volatile("movl $7, %0":"=r"(r):);return r;}' \
    'int f(void){int r;__asm__ volatile("movl %1, %0":"=r"(r):"i"(7));return r;}' \
    'int f(void){int r;__asm__ volatile("movl $1, %0":"=r"(r));return r;}' \
    'int f(char x){int r;__asm__ volatile("movsbl %b1, %0":"=r"(r):"q"(x));return r;}' \
    'int f(int x){int r;__asm__ volatile("movl %k1, %0":"=r"(r):"r"(x));return r;}' \
    'int f(short x){int r;__asm__ volatile("movswl %w1, %0":"=r"(r):"r"(x));return r;}' \
    'int f(double x){double r;__asm__ volatile("movsd %1, %0":"=x"(r):"x"(x));return (int)r;}' \
    'int f(void){int r;__asm__ volatile("xorl %0, %0":"=r"(r)::"cc");return r;}' \
    'int f(int x){int r;__asm__ volatile("movl %1, %0":"=r"(r):"U"(x));return r;}' \
    'int f(void){int r;__asm__ volatile("movl %1, %0":"=r"(r):"Ci"(7));return r;}' \
    'int f(void){int r;__asm__ volatile("movl %1, %0":"=r"(r):"M"(3));return r;}'
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

# ── %c: constant-expression operands only (gcc 16.2 wording) ─────────────────
# `%c` prints the operand as an assembly constant expression.  LCCC used to
# print the register *name*, which the kernel's BUG machinery turned into
# `.word %rcx` in `__bug_table` — silently wrong object data, not an error.
EXPECT_MESSAGE="invalid 'asm': operand is not a condition code, invalid operand code 'c'"
run_case c_on_register error \
    'int f(int x){int r;__asm__ volatile("movl %c1, %0":"=r"(r):"r"(x));return r;}'
run_case c_on_memory error \
    'int f(int x){int r;__asm__ volatile("movl %c1, %0":"=r"(r):"m"(x));return r;}'
run_case c_on_named_tie error \
    'int f(int b){int s;__asm__ volatile("movl %c1, %0":[s]"=r"(s):"[s]"(b));return s;}'
run_case c_on_immediate clean \
    'int f(void){int r;__asm__ volatile("movl %c1, %0":"=r"(r):"i"(7));return r;}'

# ── "i" operands: what gcc folds, and what it refuses ────────────────────────
# Measured (gcc 16.2, -O2): a function designator and a block-scope constant are
# folded and printed as their symbol / value; a *mutable* global object is not,
# and gcc emits both the warning and the error below.
EXPECT_MESSAGE="impossible constraint in 'asm'"
run_case symbolic_object_operand error \
    'int o; int f(int x){__asm__ volatile("addl %c1, %0":"+r"(x):"i"(o));return x;}'
run_case function_designator clean \
    'int g(void); int f(int x){__asm__ volatile("addl %c1, %0":"+r"(x):"i"(g));return x;}'
run_case block_scope_constant clean \
    'int f(int x){__auto_type k = (1 << 2);__asm__ volatile("addl %c1, %0":"+r"(x):"i"(k));return x;}'

# The folded symbol must reach the template verbatim, exactly as gcc prints it:
# `addl g, %eax`, not a register materialised through a GOT load.
printf 'int g(void); int f(int x){ __asm__ volatile("addl %%c1, %%0" : "+r"(x) : "i"(g)); return x; }\n' >"$TMP/foldsym.c"
"$CCC" -O2 -S "$TMP/foldsym.c" -o "$TMP/foldsym.s"
if ! grep -Eq '^[[:space:]]+addl[[:space:]]+g,' "$TMP/foldsym.s"; then
    echo "FAIL: a folded symbol immediate did not print as a symbol" >&2
    cat "$TMP/foldsym.s" >&2
    fail=1
fi
gcc_expect=$("$GCC" -O2 -S "$TMP/foldsym.c" -o - 2>/dev/null | grep -Ec '^[[:space:]]+addl[[:space:]]+g,')
if [[ $gcc_expect != 1 ]]; then
    echo "FAIL: the oracle no longer folds this shape (gcc printed $gcc_expect matches)" >&2
    fail=1
fi

if ((fail)); then
    echo "check_asm_operand_diagnostics: FAILED" >&2
    exit 1
fi
echo "check_asm_operand_diagnostics: PASSED"
