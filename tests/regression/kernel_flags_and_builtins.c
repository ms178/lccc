/* Compiler-interface contracts the Linux kernel build depends on.
 *
 * These are not codegen bugs but INTERFACE bugs, and they are the kind that
 * waste a day: the build either aborts on a flag every kernel passes, or -- far
 * worse -- accepts a hardening flag it does not implement and hands back a
 * binary the caller believes is protected.
 *
 *  - `-mno-<feature>` must be ACCEPTED for any ISA extension lccc never emits:
 *    a compiler that cannot generate a feature already complies with a request
 *    not to use it. The kernel probes `-mno-sse4a` before compiling anything.
 *  - `-mpreferred-stack-boundary=N` / `-mstack-alignment=N` must be accepted
 *    when N <= 16 (lccc always keeps %rsp 16-byte aligned, so a smaller
 *    request is permission, not obligation) and REJECTED above it.
 *  - `-fstack-protector*` and `-pg`/`-mfentry` must be REJECTED, because lccc
 *    emits neither canaries nor entry instrumentation. Silent acceptance is
 *    the worst possible failure mode for a security/observability feature.
 *  - C23 `typeof_unqual` must work: arch/x86/include/asm/percpu.h uses it in
 *    every per-CPU accessor.
 *  - The fortify-string builtins must exist and behave like libc.
 *  - `__has_attribute`/`__has_builtin` must expand in CODE position, not only
 *    inside `#if` (kernel/trace/trace.c computes with them in a plain
 *    expression).
 *
 * The flag behaviour is asserted by the .env-driven harness elsewhere; what is
 * checked HERE is the language-level half, at runtime, against values libc and
 * the C standard pin down.
 */
#include <stdio.h>
#include <string.h>

/* --- C23 typeof_unqual: the qualifier must be dropped. */
static int typeof_unqual_works(void)
{
       const int x = 5;
       typeof_unqual(x) y = 7;   /* must be plain `int`, hence assignable */
       y++;
       return y == 8 && x == 5;
}

/* The kernel's TYPEOF_UNQUAL shape: through a macro, on a const-qualified
 * lvalue reached by a pointer. */
#define TYPEOF_UNQUAL(x) typeof_unqual(x)
static int typeof_unqual_percpu_shape(void)
{
       const volatile long slot = 41;
       TYPEOF_UNQUAL(slot) tmp;
       tmp = slot;
       tmp++;
       return tmp == 42;
}

/* --- __has_attribute / __has_builtin in CODE position. */
static int has_macros_in_code(void)
{
       /* Exactly the kernel/trace/trace.c shape. */
       int v = 10 * 1000 + 4 * 100 + __has_attribute(__always_inline__);
       int b = __has_builtin(__builtin_expect);
       int u = __has_attribute(this_attribute_does_not_exist_xyzzy);
       /* A string containing the token must NOT be rewritten. */
       const char *s = "__has_attribute(x)";
       return v == 10401 && b == 1 && u == 0 &&
              strcmp(s, "__has_attribute(x)") == 0;
}

/* --- fortify-string builtins must match libc. */
static int fortify_builtins(void)
{
       char b[32];
       int ok = 1;

       strcpy(b, "hello");
       __builtin_strncat(b, " world!!", 6);
       ok &= (strcmp(b, "hello world") == 0);

       ok &= (__builtin_memchr("abcdef", 'd', 6) != 0);
       ok &= (__builtin_memchr("abcdef", 'z', 6) == 0);
       ok &= ((char *)__builtin_memchr("abcdef", 'd', 6) -
              (char *)"abcdef" == 3);

       ok &= (__builtin_strspn("hello", "hel") == 4);
       ok &= (__builtin_strcspn("hello", "l") == 2);
       ok &= (__builtin_strpbrk("hello", "lo") != 0);

       char d[8];
       __builtin_stpncpy(d, "ab", 4);
       ok &= (d[0] == 'a' && d[1] == 'b' && d[2] == 0);

       return ok;
}

/* --- Variadic vs non-variadic calls: %al must carry the SSE-register count
 * for a variadic callee, and the caller must be right about it. This is what
 * the "omit xorl for non-variadic" optimization must not break. */
static int variadic_sum(int n, ...)
{
       __builtin_va_list ap;
       __builtin_va_start(ap, n);
       int s = 0;
       for (int i = 0; i < n; i++)
               s += __builtin_va_arg(ap, int);
       __builtin_va_end(ap);
       return s;
}

static double variadic_fsum(int n, ...)
{
       __builtin_va_list ap;
       __builtin_va_start(ap, n);
       double s = 0;
       for (int i = 0; i < n; i++)
               s += __builtin_va_arg(ap, double);
       __builtin_va_end(ap);
       return s;
}

__attribute__((noinline)) static int plain3(int a, int b, int c)
{
       return a + b + c;
}

static int calls_ok(void)
{
       int ok = 1;
       /* variadic, integer only: %al must be 0 */
       ok &= (variadic_sum(4, 1, 2, 3, 4) == 10);
       /* variadic with FP: %al must be the live SSE count */
       ok &= (variadic_fsum(3, 1.5, 2.5, 3.0) == 7.0);
       /* non-variadic through a function pointer: no %al setup needed */
       int (*fp)(int, int, int) = plain3;
       ok &= (fp(1, 2, 3) == 6);
       return ok;
}

struct check { const char *name; int ok; };

int main(void)
{
       struct check c[] = {
               { "typeof_unqual",             typeof_unqual_works() },
               { "typeof_unqual_percpu",      typeof_unqual_percpu_shape() },
               { "has_macros_in_code",        has_macros_in_code() },
               { "fortify_builtins",          fortify_builtins() },
               { "calls_ok",                  calls_ok() },
       };
       int fail = 0;
       for (unsigned i = 0; i < sizeof(c) / sizeof(c[0]); i++) {
               if (!c[i].ok) {
                       printf("FAIL %s\n", c[i].name);
                       fail = 1;
               }
       }
       if (fail)
               return 1;
       printf("PASS kernel_flags_and_builtins\n");
       return 0;
}
