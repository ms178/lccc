#!/usr/bin/env bash
# LCCC vs GCC compatibility suite: real-world C patterns, simple to complex,
# compared by RUNNING both compilers' output and diffing stdout (a program that
# compiles but prints the wrong thing is exactly the class the corpus' compile-
# only legs cannot see).
#
# AUD-HYGIENE-1: this suite used to live at tests/compat_test.sh, unreferenced by
# any workflow, writing through fixed `/tmp/lccc_test*` paths and always exiting
# 0 -- so it could not fail anything, and two concurrent runs clobbered each
# other's binaries.  It is a gate now: private scratch directory, the compiler
# under test comes from $CCC (the repo's gate convention), and the exit status
# reports the verdict.
#
# Usage: CCC=target/fastbuild/lccc bash tests/regression/check_compat_suite.sh
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
GCC_INC="-I$(gcc -print-file-name=include)"
LCCC="$ccc $GCC_INC -O2"
GCC="gcc -O2"
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
PASS=0
FAIL=0
SKIP=0
TOTAL=0

# test_program <name> <source> -- compile and run the program with GCC and with
# LCCC, and require identical stdout+stderr.  There is no third "expected
# output" argument: GCC's output IS the expectation, so a test can never pass by
# agreeing with a hand-written string that both compilers disagree with.
test_program() {
    local name="$1"
    local src="$2"
    TOTAL=$((TOTAL + 1))

    printf '%s\n' "$src" > "$td/test.c"

    # Compile with GCC (reference)
    if ! $GCC -o "$td/gcc" "$td/test.c" -lm 2>"$td/gcc.err"; then
        echo "SKIP  $name (GCC compile failed)"
        SKIP=$((SKIP + 1))
        return
    fi

    # Get GCC output
    local gcc_out
    gcc_out=$("$td/gcc" 2>&1 || true)

    # Compile with LCCC
    if ! $LCCC -o "$td/lccc" "$td/test.c" -lm 2>"$td/lccc.err"; then
        echo "FAIL  $name (LCCC compile failed)"
        sed 's/^/      /' "$td/lccc.err" >&2
        FAIL=$((FAIL + 1))
        return
    fi

    # Get LCCC output
    local lccc_out
    lccc_out=$("$td/lccc" 2>&1 || true)

    if [ "$gcc_out" = "$lccc_out" ]; then
        echo "PASS  $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL  $name"
        echo "  GCC:  $gcc_out"
        echo "  LCCC: $lccc_out"
        FAIL=$((FAIL + 1))
    fi
}

echo "=== LCCC Compatibility Tests ==="
echo ""

# --- Level 1: Basic C ---
echo "--- Level 1: Basic C ---"

test_program "hello world" '
#include <stdio.h>
int main() { printf("hello world\n"); return 0; }
'

test_program "arithmetic" '
#include <stdio.h>
int main() { printf("%d %d %d\n", 2+3, 10*7, 100/3); return 0; }
'

test_program "string ops" '
#include <stdio.h>
#include <string.h>
int main() {
    char buf[100] = "hello";
    strcat(buf, " world");
    printf("%s len=%zu\n", buf, strlen(buf));
    return 0;
}
'

test_program "for loop" '
#include <stdio.h>
int main() {
    int sum = 0;
    for (int i = 1; i <= 100; i++) sum += i;
    printf("sum=%d\n", sum);
    return 0;
}
'

test_program "while + break" '
#include <stdio.h>
int main() {
    int n = 1;
    while (n < 1000) { n *= 2; if (n > 500) break; }
    printf("n=%d\n", n);
    return 0;
}
'

# --- Level 2: Functions and Pointers ---
echo ""
echo "--- Level 2: Functions and Pointers ---"

test_program "recursion" '
#include <stdio.h>
int fib(int n) { return n <= 1 ? n : fib(n-1) + fib(n-2); }
int main() { printf("fib(10)=%d\n", fib(10)); return 0; }
'

test_program "function pointers" '
#include <stdio.h>
int add(int a, int b) { return a + b; }
int mul(int a, int b) { return a * b; }
int main() {
    int (*ops[])(int,int) = {add, mul};
    printf("%d %d\n", ops[0](3,4), ops[1](3,4));
    return 0;
}
'

test_program "pointer arithmetic" '
#include <stdio.h>
int main() {
    int arr[] = {10, 20, 30, 40, 50};
    int *p = arr + 2;
    printf("%d %d %d\n", *p, *(p-1), *(p+2));
    return 0;
}
'

test_program "malloc/free" '
#include <stdio.h>
#include <stdlib.h>
int main() {
    int *p = malloc(10 * sizeof(int));
    for (int i = 0; i < 10; i++) p[i] = i * i;
    int sum = 0;
    for (int i = 0; i < 10; i++) sum += p[i];
    free(p);
    printf("sum=%d\n", sum);
    return 0;
}
'

# --- Level 3: Structs ---
echo ""
echo "--- Level 3: Structs ---"

test_program "basic struct" '
#include <stdio.h>
typedef struct { int x, y; } Point;
int main() {
    Point p = {3, 4};
    printf("(%d,%d)\n", p.x, p.y);
    return 0;
}
'

test_program "struct array" '
#include <stdio.h>
typedef struct { int x, y; } Point;
int main() {
    Point pts[] = {{1,2}, {3,4}, {5,6}};
    int sum = 0;
    for (int i = 0; i < 3; i++) sum += pts[i].x + pts[i].y;
    printf("sum=%d\n", sum);
    return 0;
}
'

test_program "struct with function pointer" '
#include <stdio.h>
typedef struct { const char *name; int (*func)(int); } Op;
static int dbl(int x) { return x * 2; }
static int sqr(int x) { return x * x; }
int main() {
    Op ops[] = {{"dbl", dbl}, {"sqr", sqr}, {0, 0}};
    for (int i = 0; ops[i].name; i++)
        printf("%s(%d)=%d\n", ops[i].name, 5, ops[i].func(5));
    return 0;
}
'

test_program "linked list" '
#include <stdio.h>
#include <stdlib.h>
typedef struct Node { int val; struct Node *next; } Node;
int main() {
    Node *head = NULL;
    for (int i = 5; i >= 1; i--) {
        Node *n = malloc(sizeof(Node));
        n->val = i; n->next = head; head = n;
    }
    int sum = 0;
    for (Node *p = head; p; p = p->next) sum += p->val;
    printf("sum=%d\n", sum);
    // leak is ok for test
    return 0;
}
'

# --- Level 4: Complex patterns ---
echo ""
echo "--- Level 4: Complex patterns ---"

test_program "switch statement" '
#include <stdio.h>
int classify(int n) {
    switch (n % 4) {
        case 0: return 100;
        case 1: return 200;
        case 2: return 300;
        default: return 400;
    }
}
int main() {
    int sum = 0;
    for (int i = 0; i < 20; i++) sum += classify(i);
    printf("sum=%d\n", sum);
    return 0;
}
'

test_program "qsort callback" '
#include <stdio.h>
#include <stdlib.h>
int cmp(const void *a, const void *b) { return *(int*)a - *(int*)b; }
int main() {
    int arr[] = {5, 2, 8, 1, 9, 3, 7, 4, 6, 0};
    qsort(arr, 10, sizeof(int), cmp);
    for (int i = 0; i < 10; i++) printf("%d ", arr[i]);
    printf("\n");
    return 0;
}
'

test_program "varargs" '
#include <stdio.h>
#include <stdarg.h>
int sum_n(int n, ...) {
    va_list ap;
    va_start(ap, n);
    int s = 0;
    for (int i = 0; i < n; i++) s += va_arg(ap, int);
    va_end(ap);
    return s;
}
int main() { printf("sum=%d\n", sum_n(4, 10, 20, 30, 40)); return 0; }
'

test_program "inline asm percent clobber" '
#include <stdio.h>
static int clobber_test(int in) {
    int out = -1;
#if defined(__x86_64__)
    asm volatile (
        "mov %1, %0\n\t"
        "mov $99, %%ecx"
        : "=&r"(out)
        : "r"(in)
        : "%rcx");
#elif defined(__i386__)
    asm volatile (
        "mov %1, %0\n\t"
        "mov $99, %%ecx"
        : "=&r"(out)
        : "r"(in)
        : "%ecx");
#else
    out = in;
#endif
    return out;
}
int main() { printf("%d\n", clobber_test(7)); return 0; }
'

test_program "inline asm readwrite local" '
#include <stdio.h>
int main() {
    int x = 5;
#if defined(__x86_64__) || defined(__i386__)
    asm volatile ("addl $3, %0" : "+r"(x));
#endif
    printf("%d\n", x);
    return 0;
}
'

test_program "setjmp/longjmp" '
#include <stdio.h>
#include <setjmp.h>
jmp_buf env;
void bail(int code) { longjmp(env, code); }
int main() {
    int v = setjmp(env);
    if (v == 0) { bail(42); }
    printf("caught %d\n", v);
    return 0;
}
'

test_program "floating point" '
#include <stdio.h>
#include <math.h>
int main() {
    double x = 3.14159265;
    printf("sin=%.4f cos=%.4f sqrt2=%.4f\n", sin(x), cos(x), sqrt(2.0));
    return 0;
}
'

# A GNU vector comparison is a mask of signed integer lanes as wide as the
# operand lanes (int lanes for float, 64-bit lanes for double); __typeof__,
# __auto_type and sizeof must see that vector type.  LCCC used to lower float
# lane comparisons as a compare of the vector's address (segfault), type the
# comparison as int (sizeof 4, m[1] crashed) and collapse typeof/auto_type
# vectors to their lane type.
test_program "GNU vector comparison masks" '
#include <stdio.h>
#include <math.h>
typedef float v4f __attribute__((vector_size(16)));
typedef double v2d __attribute__((vector_size(16)));
typedef int v4i __attribute__((vector_size(16)));
typedef unsigned char v16u __attribute__((vector_size(16)));
typedef short v8s __attribute__((vector_size(16)));
typedef float v8f __attribute__((vector_size(32)));
static void pf(const char *n, v4i m) { printf("%s %d %d %d %d\n", n, m[0], m[1], m[2], m[3]); }
int main(void) {
  volatile float nanv = NAN;
  v4f f = {1, 2, nanv, -3}, g = {1, 1, 1, nanv};
  pf("eq", f == g); pf("ne", f != g); pf("lt", f < g); pf("le", f <= g);
  pf("gt", f > g); pf("ge", f >= g); pf("lt1", f < 1); pf("1lt", 1 < f);
  pf("ltd", f < 1.5);
  v2d d = {0.5, nanv}, e = {0.5, 0.5};
  __typeof__(d == e) dm = d == e; __typeof__(d != e) dn = d != e;
  printf("d %lld %lld %lld %lld %zu %zu\n", (long long)dm[0], (long long)dm[1], (long long)dn[0], (long long)dn[1], sizeof dm, sizeof(d < 0));
  v8f h = {1,2,3,4,5,6,7,8}; __typeof__(h > 4) hm = h > 4;
  printf("h %zu %d %d %d\n", sizeof hm, hm[0], hm[4], hm[7]);
  v16u a = {1, 200, 3}, b = {1, 2, 255};
  __typeof__(a < b) am = a < b;
  printf("u %zu %d %d %d %d\n", sizeof am, am[0], am[1], am[2], am[3]);
  v8s s = {-1, 5, 7}, t = {0, 5, -7};
  __auto_type sm = s >= t;
  printf("s %zu %d %d %d\n", sizeof sm, sm[0], sm[1], sm[2]);
  v4i i = {1, -2, 3, 4};
  pf("ilt", i < 2); pf("not", ~(i == i)); pf("neg", -(f < g));
  v4f sel = (v4f)((v4i)f & (f > 0));
  printf("sel %g %g %g\n", sel[0], sel[1], sel[3]);
  return 0;
}
'

# --- Summary ---
echo ""
echo "=== Results: $PASS passed, $FAIL failed, $SKIP skipped (of $TOTAL) ==="
[ "$FAIL" -eq 0 ] || { echo "FAIL: $FAIL compatibility test(s) disagreed with GCC" >&2; exit 1; }
[ "$PASS" -gt 0 ] || { echo "FAIL: no compatibility test ran" >&2; exit 1; }
echo "PASS: every compatibility program matched GCC"
