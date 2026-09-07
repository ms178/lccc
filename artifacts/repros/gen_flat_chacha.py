"""Generate a faithful chacha20_core whose copy loops are hand-unrolled.

This is the shape the loop unroller would produce, so it isolates the effect of
aggregate_sroa's constant-offset splitting (form 4) from the question of
whether the 16-iteration copy loops get unrolled at all.
"""
import sys


def rotl(v, n):
    return "(((%s) << %d) | ((%s) >> %d))" % (v, n, v, 32 - n)


def qr(a, b, c, d):
    return "\n    ".join(
        [
            "%s += %s;" % (a, b),
            "%s ^= %s;" % (d, a),
            "%s = %s;" % (d, rotl(d, 16)),
            "%s += %s;" % (c, d),
            "%s ^= %s;" % (b, c),
            "%s = %s;" % (b, rotl(b, 12)),
            "%s += %s;" % (a, b),
            "%s ^= %s;" % (d, a),
            "%s = %s;" % (d, rotl(d, 8)),
            "%s += %s;" % (c, d),
            "%s ^= %s;" % (b, c),
            "%s = %s;" % (b, rotl(b, 7)),
        ]
    )


COL = [(0, 4, 8, 12), (1, 5, 9, 13), (2, 6, 10, 14), (3, 7, 11, 15)]
DIAG = [(0, 5, 10, 15), (1, 6, 11, 12), (2, 7, 8, 13), (3, 4, 9, 14)]

out = []
ROUNDS = int(sys.argv[1]) if len(sys.argv) > 1 else 10
out.append("#include <stdio.h>")
out.append("typedef unsigned int u32;")
out.append("__attribute__((noinline)) static void chacha20_core(u32 out[16], const u32 in[16]) {")
out.append("  u32 x[16];")
for i in range(16):
    out.append("  x[%d] = in[%d];" % (i, i))
for r in range(ROUNDS):
    for (a, b, c, d) in COL:
        out.append("  " + qr("x[%d]" % a, "x[%d]" % b, "x[%d]" % c, "x[%d]" % d))
    for (a, b, c, d) in DIAG:
        out.append("  " + qr("x[%d]" % a, "x[%d]" % b, "x[%d]" % c, "x[%d]" % d))
for i in range(16):
    out.append("  out[%d] = x[%d] + in[%d];" % (i, i, i))
out.append("}")
out.append(
    "static const u32 TIN[16] = {0x61707865,0x3320646e,0x79622d32,0x6b206574,"
    "0x03020100,0x07060504,0x0b0a0908,0x0f0e0d0c,0x13121110,0x17161514,"
    "0x1b1a1918,0x1f1e1d1c,0x00000001,0x09000000,0x4a000000,0x00000000};"
)
out.append("int main(void) {")
out.append("  u32 o[16];")
out.append("  for (int p = 0; p < 64; p++) chacha20_core(o, TIN);")
out.append('  printf("%08x%08x\\n", o[0], o[15]);')
out.append("  return o[0] != 0xe4e7f110;")
out.append("}")
sys.stdout.write("\n".join(out) + "\n")
