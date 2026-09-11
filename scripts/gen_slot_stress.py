#!/usr/bin/env python3
"""Generate deterministic C programs that stress stack-slot allocation.

Why this exists
---------------
The -O2 preboot-ZSTD miscompile was a *stack layout* bug (a value whose slot
was narrower than the width the emitter reloaded it with, and hole-aware slot
sharing that handed one value's slot to a neighbour). Bugs of that class are
almost invisible in small unit tests: they need

  * many simultaneously live values, so values actually spill (few registers
    are free and every call clobbers the caller-saved set);
  * a mix of widths (1/2/4/8/12/16/32 bytes), so slot *sizing* and
    width-partitioned 4-byte slots are exercised;
  * control flow (if/else diamonds, loops, switch), so live ranges have holes
    and convex hulls, which is exactly what Tier-2 (graph-coloured) slot
    sharing keys on;
  * values that are live across a call, across a loop backedge, or defined on
    only one arm and used after the join (phi nodes with partial defs);
  * address-taken / volatile / setjmp / alloca / inline-asm values, which must
    never share a slot with an unrelated value.

The generator emits strictly well-defined C: unsigned arithmetic only, no
division by zero, no shifts past the width, no signed overflow, no
uninitialised reads. Any divergence against GCC is therefore a real
miscompile, never a UB artefact. The program prints one 64-bit checksum.

Usage:
    gen_slot_stress.py SEED > case.c
"""

import random
import sys

TYPES = [
    # (c type, printf/format, how to make a deterministic value, byte width)
    ("unsigned char", "u8", 1),
    ("unsigned short", "u16", 2),
    ("unsigned int", "u32", 4),
    ("unsigned long long", "u64", 8),
    ("long double", "ldbl", 16),
    ("struct S12", "s12", 12),
    ("struct S3", "s3", 3),
    ("struct S24", "s24", 24),
    ("unsigned long long __attribute__((vector_size(16)))", "v16", 16),
]

PRELUDE = r"""
#include <stdio.h>
#include <string.h>
#include <setjmp.h>
#include <stdlib.h>

struct S3  { unsigned char b[3]; };
struct S12 { unsigned int a, b, c; };
struct S24 { unsigned long long x, y, z; };

static volatile int g_vol;                 /* anti-folding, forces reloads */
static unsigned long long g_sink;          /* consumes every computed value */

/* Noinline external barrier: clobbers caller-saved registers, so every value
   that is live across it must be spilled to a stack slot. */
__attribute__((noinline)) unsigned long long ext(unsigned long long x)
{
    g_vol++;
    return x ^ 0x9e3779b97f4a7c15ULL ^ (unsigned long long)g_vol;
}

__attribute__((noinline)) void barrier(void) { g_vol += 3; }

static unsigned long long mix(unsigned long long acc, unsigned long long v)
{
    acc ^= v + 0x9e3779b97f4a7c15ULL + (acc << 6u) + (acc >> 2u);
    return acc;
}
"""


class Gen:
    def __init__(self, seed, n_values=28, n_regions=6):
        self.r = random.Random(seed)
        self.n_values = n_values
        self.n_regions = n_regions
        self.lines = []
        self.counter = 0
        self.declared = set()

    def emit(self, indent, text):
        self.lines.append("    " * indent + text)

    def val(self, kind, tag):
        """A deterministic, in-range value of the given kind."""
        self.counter += 1
        c = self.counter
        if kind == "u8":
            return "((unsigned char)(%du * 7u + 3u))" % c
        if kind == "u16":
            return "((unsigned short)(%du * 3121u + 17u))" % c
        if kind == "u32":
            return "((unsigned int)(%du * 2654435761u + 12345u))" % c
        if kind == "u64":
            return "((unsigned long long)(%dull * 0x9e3779b97f4a7c15ULL + 7ULL))" % c
        if kind == "s3":
            return "((struct S3){ { (unsigned char)(%du * 5u), (unsigned char)(%du * 9u), (unsigned char)(%du * 13u) } })" % (
                c, c + 1, c + 2)
        if kind == "s12":
            return "((struct S12){ %du * 3u, %du * 11u + 1u, %du * 13u + 2u })" % (c, c, c)
        if kind == "s24":
            return "((struct S24){ %dull * 5ull, %dull * 7ull + 1ull, %dull * 9ull + 2ull })" % (c, c, c)
        if kind == "v16":
            return "((unsigned long long __attribute__((vector_size(16)))){ %dull * 3ull, %dull * 5ull + 1ull })" % (c, c)
        if kind == "ldbl":
            return "((long double)(%d) * 1.25L + 0.5L)" % c
        raise AssertionError(kind)

    def consume(self, indent, name, kind):
        """Fold `name` into the global sink, whatever its type."""
        if kind in ("u8", "u16", "u32", "u64"):
            self.emit(indent, "g_sink = mix(g_sink, (unsigned long long)%s);" % name)
        elif kind == "ldbl":
            self.emit(indent, "g_sink = mix(g_sink, (unsigned long long)((long long)%s));" % name)
        elif kind == "s3":
            self.emit(indent,
                      "g_sink = mix(g_sink, (unsigned long long)((unsigned)%s.b[0] * 3u + %s.b[1] * 5u + %s.b[2] * 7u));"
                      % (name, name, name))
        elif kind == "s12":
            self.emit(indent,
                      "g_sink = mix(g_sink, (unsigned long long)((unsigned long long)%s.a * 3ull + (unsigned long long)%s.b * 5ull + (unsigned long long)%s.c * 7ull));"
                      % (name, name, name))
        elif kind == "s24":
            self.emit(indent,
                      "g_sink = mix(g_sink, %s.x * 3ull + %s.y * 5ull + %s.z * 7ull);" % (name, name, name))
        elif kind == "v16":
            self.emit(indent,
                      "g_sink = mix(g_sink, (unsigned long long)((unsigned long long __attribute__((vector_size(16))))%s)[0] ^ ((unsigned long long __attribute__((vector_size(16))))%s)[1]);"
                      % (name, name))

    def declare(self, indent, name, ctype):
        self.emit(indent, "%s %s;" % (ctype, name))

    def make_value(self, indent, idx):
        ctype, kind, width = self.r.choice(TYPES)
        name = "v%d_%d" % (idx, len(self.declared))
        if name in self.declared:
            return None
        self.declared.add(name)
        self.declare(indent, name, ctype)
        self.emit(indent, "%s = %s;" % (name, self.val(kind, name)))
        return (name, kind, width)

    # ------------------------------ region shapes ---------------------------
    def region_diamond(self, indent, depth):
        """if/else: values defined in one arm, consumed after the join."""
        n = self.r.randint(2, 5)
        vals = [self.make_value(indent, i) for i in range(n)]
        vals = [v for v in vals if v]
        cond = "((g_sink >> %du) & 1u) ^ %d" % (self.r.randint(0, 8), self.r.randint(0, 1))
        self.emit(indent, "if (%s) {" % cond)
        for (name, kind, _w) in vals:
            if self.r.random() < 0.5:
                self.emit(indent + 1, "%s = %s;" % (name, self.val(kind, name)))
            self.consume(indent + 1, name, kind)
        self.emit(indent + 1, "ext(g_sink);")
        self.emit(indent, "} else {")
        for (name, kind, _w) in vals:
            if self.r.random() < 0.5:
                self.emit(indent + 1, "%s = %s;" % (name, self.val(kind, name)))
            self.consume(indent + 1, name, kind)
        self.emit(indent + 1, "barrier();")
        self.emit(indent, "}")
        self.emit(indent, "ext(g_sink);")
        for (name, kind, _w) in vals:
            self.consume(indent, name, kind)

    def region_loop(self, indent, depth):
        """Loop-carried values: live across the backedge (fat hull)."""
        n = self.r.randint(2, 4)
        vals = [self.make_value(indent, i) for i in range(n)]
        vals = [v for v in vals if v]
        trip = self.r.randint(2, 5)
        self.emit(indent, "for (int i%d = 0; i%d < %d; i%d++) {" % (depth, depth, trip, depth))
        for (name, kind, _w) in vals:
            self.emit(indent + 1, "%s = %s;" % (name, self.val(kind, name)))
        self.emit(indent + 1, "ext(g_sink); barrier();")
        for (name, kind, _w) in vals:
            self.consume(indent + 1, name, kind)
        self.emit(indent + 1, "g_sink = mix(g_sink, (unsigned long long)i%d);" % depth)
        self.emit(indent, "}")
        for (name, kind, _w) in vals:
            self.consume(indent, name, kind)

    def region_switch(self, indent, depth):
        """switch: three arms, values defined per arm, used after the join."""
        n = self.r.randint(2, 5)
        vals = [self.make_value(indent, i) for i in range(n)]
        vals = [v for v in vals if v]
        self.emit(indent, "switch ((int)((g_sink >> %du) & 3u)) {" % self.r.randint(0, 6))
        for case in range(3):
            self.emit(indent, "case %d: {" % case)
            for (name, kind, _w) in vals:
                if self.r.random() < 0.6:
                    self.emit(indent + 1, "%s = %s;" % (name, self.val(kind, name)))
                self.consume(indent + 1, name, kind)
            self.emit(indent + 1, "barrier(); break;")
            self.emit(indent, "}")
        self.emit(indent, "default: barrier(); break;")
        self.emit(indent, "}")
        for (name, kind, _w) in vals:
            self.consume(indent, name, kind)

    def region_pressure(self, indent, depth):
        """Many live values across a call: forces real spilling."""
        n = self.r.randint(10, self.n_values)
        vals = []
        for i in range(n):
            v = self.make_value(indent, i)
            if v:
                vals.append(v)
        self.emit(indent, "ext(g_sink); barrier(); ext(g_sink);")
        for (name, kind, _w) in vals:
            self.consume(indent, name, kind)

    def region_aliased(self, indent, depth):
        """Address-taken / volatile / alloca values must not share slots."""
        self.emit(indent, "{")
        self.emit(indent + 1, "unsigned long long stack_a = %s;" % self.val("u64", "sa"))
        self.emit(indent + 1, "unsigned int stack_b = %s;" % self.val("u32", "sb"))
        self.emit(indent + 1, "unsigned long long *pa = &stack_a;")
        self.emit(indent + 1, "unsigned int *pb = &stack_b;")
        self.emit(indent + 1, "volatile unsigned long long vv = %s;" % self.val("u64", "vv"))
        self.emit(indent + 1, "barrier(); ext(*pa); ext(*pb);")
        self.emit(indent + 1, "g_sink = mix(g_sink, *pa);")
        self.emit(indent + 1, "g_sink = mix(g_sink, (unsigned long long)*pb);")
        self.emit(indent + 1, "g_sink = mix(g_sink, (unsigned long long)vv);")
        self.emit(indent + 1, "barrier();")
        self.emit(indent + 1, "g_sink = mix(g_sink, *pa + (unsigned long long)*pb + (unsigned long long)vv);")
        self.emit(indent, "}")

    def region_asm(self, indent, depth):
        """Inline asm with a memory clobber: no value may live in a shared
        slot across it unless the slot is reloaded afterwards."""
        self.emit(indent, "{")
        self.emit(indent + 1, "unsigned long long a1 = %s;" % self.val("u64", "a1"))
        self.emit(indent + 1, "unsigned int a2 = %s;" % self.val("u32", "a2"))
        self.emit(indent + 1, 'asm volatile("" : "+r"(a1), "+r"(a2) :: "memory");')
        self.emit(indent + 1, "barrier();")
        self.emit(indent + 1, "g_sink = mix(g_sink, a1 ^ (unsigned long long)a2);")
        self.emit(indent, "}")

    def region_setjmp(self, indent, depth):
        """setjmp/longjmp: values live across the resume point must not be in
        a slot that another value reused while the frame was active."""
        self.emit(indent, "{")
        self.emit(indent + 1, "static jmp_buf jb;")
        # s1/s2 MUST be volatile: they are modified between setjmp and
        # longjmp and read again after the resume, and C11 7.13.2.1 makes
        # non-volatile automatic objects so modified *indeterminate* after a
        # longjmp. gcc -O0 keeps such locals memory-backed and reads the
        # modified value; a compiler that re-materialises them from a register
        # copy (lccc -O0) reads the stale one — both conform, so a non-volatile
        # oracle cannot demand exact equality. volatile forces every access to
        # the stack home, which is exactly the slot-reuse-safety property this
        # region exists to test.
        self.emit(indent + 1, "volatile unsigned long long s1 = %s;" % self.val("u64", "s1"))
        self.emit(indent + 1, "volatile unsigned int s2 = %s;" % self.val("u32", "s2"))
        self.emit(indent + 1, "if (setjmp(jb) == 0) {")
        self.emit(indent + 2, "s1 = %s;" % self.val("u64", "s1b"))
        self.emit(indent + 2, "s2 = %s;" % self.val("u32", "s2b"))
        self.emit(indent + 2, "barrier();")
        self.emit(indent + 2, "g_sink = mix(g_sink, s1 + (unsigned long long)s2);")
        self.emit(indent + 2, "longjmp(jb, 1);")
        self.emit(indent + 1, "}")
        self.emit(indent + 1, "g_sink = mix(g_sink, s1 * 3ull + (unsigned long long)s2 * 5ull);")
        self.emit(indent, "}")

    def region_vla(self, indent, depth):
        self.emit(indent, "{")
        self.emit(indent + 1, "unsigned n = %u;" % self.r.randint(4, 24))
        self.emit(indent + 1, "unsigned long long a[n];")
        self.emit(indent + 1, "unsigned int b[n];")
        self.emit(indent + 1, "for (unsigned i = 0; i < n; i++) { a[i] = %s + i; b[i] = (unsigned)(%s) + i; }"
                  % (self.val("u64", "va"), self.val("u32", "vb")))
        self.emit(indent + 1, "barrier();")
        self.emit(indent + 1, "for (unsigned i = 0; i < n; i++) g_sink = mix(g_sink, a[i] + (unsigned long long)b[i]);")
        self.emit(indent, "}")

    def region_bittest(self, indent, depth):
        """Bit extraction and bit-test branches on a runtime value.

        `(x >> k) & 1` is canonicalized to a cross-target BitTest and lowered
        with x86 BT. A zero index must still yield bit zero of x — folding
        `BitTest(base, 0)` to constant 0 was a real miscompile that made an
        `if (((x >> 0) & 1) ^ 1)` take the wrong arm.
        """
        self.emit(indent, "{")
        self.emit(indent + 1, "volatile unsigned long long bt = %s;" % self.val("u64", "bt"))
        self.emit(indent + 1, "unsigned long long x = bt;")
        for k in range(0, 8):
            self.emit(indent + 1,
                      "g_sink = mix(g_sink, (unsigned long long)((x >> %du) & 1u) * %dull);" % (k, k + 1))
        self.emit(indent + 1, "barrier();")
        self.emit(indent + 1, "if (((x >> 0u) & 1u) ^ 1u) { g_sink = mix(g_sink, 101ull); barrier(); }")
        self.emit(indent + 1, "else { g_sink = mix(g_sink, 202ull); ext(g_sink); }")
        self.emit(indent + 1, "if (((x >> 3u) & 1u) ^ 0u) { g_sink = mix(g_sink, 303ull); ext(g_sink); }")
        self.emit(indent + 1, "else { g_sink = mix(g_sink, 404ull); barrier(); }")
        self.emit(indent + 1, "g_sink = mix(g_sink, (unsigned long long)((x >> 0u) & 1u));")
        self.emit(indent, "}")

    REGIONS = [region_diamond, region_loop, region_switch, region_pressure,
               region_aliased, region_asm, region_setjmp, region_vla,
               region_bittest]

    def build(self):
        self.lines.append(PRELUDE)
        self.lines.append("int main(void)\n{")
        self.emit(1, "g_sink = 0x123456789abcdefULL;")
        for d in range(self.n_regions):
            r = self.r.choice(self.REGIONS)
            r(self, 1, d)
            self.emit(1, "g_sink = mix(g_sink, %dull);" % d)
        self.emit(1, 'printf("%llu\\n", g_sink);')
        self.emit(1, "return 0;")
        self.lines.append("}")
        return "\n".join(self.lines) + "\n"


def main():
    seed = int(sys.argv[1]) if len(sys.argv) > 1 else 1
    n_values = int(sys.argv[2]) if len(sys.argv) > 2 else 28
    n_regions = int(sys.argv[3]) if len(sys.argv) > 3 else 6
    sys.stdout.write(Gen(seed, n_values, n_regions).build())


if __name__ == "__main__":
    main()
