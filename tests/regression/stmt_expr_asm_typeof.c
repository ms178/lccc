// Regression reproducer for the kernel B2 blocker: statement-expression
// + inline-asm output operand + typeof() unevaluated context.
//
// The kernel's net/core/sock.o xchg/cmpxchg patterns reduce to this shape.
// `({ typeof(*({...})) *p = ...; ((typeof(*({...})) *)(u)); })` where the
// inner stmt-expr contains an inline asm with a "+r"(ret) output operand.
//
// The inferred type of the outer stmt-expr should be the pointer type
// declared via typeof, NOT `int *` (the asm output operand's default).

#include <stddef.h>

struct dst_entry { int x; };

static struct dst_entry global_dst = { 42 };

#define __xchg(ptr, size)                                  \
    ({                                                     \
        struct dst_entry *__ai_ptr = (struct dst_entry *)(ptr); \
        struct dst_entry __ret;                             \
        switch (size) {                                    \
            case 8:                                        \
                asm volatile("" : "+r"(__ret), "+m"(*__ai_ptr)); \
                break;                                     \
        }                                                  \
        __ret;                                             \
    })

int main(void) {
    struct dst_entry *p = &global_dst;
    // The outer stmt-expr's last expression is `__ret` (struct dst_entry).
    // typeof of that is `struct dst_entry`. The cast `(typeof(...)*)(p)`
    // is `struct dst_entry *`. q->x should be 42.
    typeof(({ __xchg(p, 8); })) *q = (typeof(({ __xchg(p, 8); })) *)(p);
    // The runner protocol requires exit 0: success is q pointing at
    // global_dst with the expected payload, not the payload itself
    // (the original form returned 42 == q->x, failing the exit-0 check
    // for every compiler).
    return (q == &global_dst && q->x == 42) ? 0 : -1;
}
