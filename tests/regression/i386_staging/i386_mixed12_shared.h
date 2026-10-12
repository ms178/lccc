/* The shared half of the cross-compiler ABI test: one struct, one declaration,
 * compiled twice -- once by the compiler under test and once by the oracle -- and
 * linked together.  A calling convention is only *correct* at the boundary between
 * two independent compilers, so this header exists to make the two TUs state the
 * same type without either one seeing the other's source.
 *
 * The struct is deliberately 12 bytes and packed: eight bytes of INTEGER plus four
 * of SSE, which is the size at which the x86-64 psABI stops being able to call the
 * whole thing an integer, and the point at which a compiler that has no SSE class for
 * an argument falls back to MEMORY.  `use` returns the folded value so a mismatched
 * boundary is visible as a wrong number rather than a crash.
 */
#ifndef I386_MIXED12_SHARED_H
#define I386_MIXED12_SHARED_H

struct P {
    unsigned long long w;
    float f;
} __attribute__((packed));

unsigned long long use(struct P p);

#endif /* I386_MIXED12_SHARED_H */
