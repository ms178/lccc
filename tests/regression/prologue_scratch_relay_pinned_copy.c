/* A 64-bit scratch copy of a sign-extended parameter, staged through an
 * argument register and copied into a callee-saved register in the entry
 * block, looks exactly like a parameter pre-store to the peephole prologue
 * pinner:
 *
 *     movslq %edi, %r9        # (long)x, r9 is just a scratch here
 *     movq   %r9, %rbx        # pinned as a "pre-store" (never deleted)
 *     shlq   $2, %rbx
 *
 * `fold_movslq_relay` retargeted the extension to %rbx and asked `mark_nop`
 * to delete the copy; the pin made `mark_nop` refuse AFTER the rewrite, so
 * the output read `movslq %edi, %rbx; movq %r9, %rbx` and %rbx came from an
 * undefined %r9 (gcc.c-torture/execute/postmod-1.c at -O1, SIGSEGV).
 *
 * Six post-modified pointers plus five register-hungry volatile adds keep
 * enough values live that the allocator stages one index through %r9.
 */
#include <stdio.h>

#define MANY(A) A(0), A(1), A(2), A(3), A(4), A(5)
#define DECLARE_ARRAY(A) array##A[0x10]
#define DECLARE_COUNTER(A) counter##A = 0
#define DECLARE_POINTER(A) *pointer##A = array##A + x
#define BEFORE(A) counter##A += *pointer##A, pointer##A += 3
#define AFTER(A) counter##A += pointer##A[x]
#define INIT_ARRAY(A) array##A[1] = 1.0f, array##A[5] = 2.0f
#define MANY2(A) A(0), A(1), A(2), A(3), A(4)
#define INIT_VOLATILE(A) addend##A = vol
#define ADD_VOLATILE(A) vol += addend##A

float MANY(DECLARE_ARRAY);
float MANY(DECLARE_COUNTER);
volatile int stop = 1;
volatile int vol;

__attribute__((noinline)) void foo(int x) {
    float MANY(DECLARE_POINTER);
    int i;
    do {
        MANY(BEFORE);
        MANY(AFTER);
        {
            int MANY2(INIT_VOLATILE);
            for (i = 0; i < 10; i++)
                MANY2(ADD_VOLATILE);
        }
    } while (!stop);
}

int main(void) {
    MANY(INIT_ARRAY);
    foo(1);
    printf("%g %g %g %g %g %g\n", counter0, counter1, counter2, counter3,
           counter4, counter5);
    return !(counter0 == 3.0f && counter1 == 3.0f && counter2 == 3.0f &&
             counter3 == 3.0f && counter4 == 3.0f && counter5 == 3.0f);
}
