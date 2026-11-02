/* S12 regression pin (i686, all opt levels): `!!c` on a float used to lower
 * to the x86-64 truthiness bit-trick (And(bits, 0x7fffffff) at I64 width)
 * even on ILP32 targets. Materializing the I64 bit pattern of a 4-byte F32
 * param wrote a synthesized zero high half at param_slot+4 — past the
 * parameter, over the return address — so `!!c * 7LL == 0` jumped to zero
 * (gcc.c-torture 20080529-1, PR target/36362). On 32-bit targets both F32
 * and F64 truthiness now lower to a real float compare against zero.
 * Expected exit 0. */

extern void abort (void);

int
test (float c)
{
  return !!c * 7LL == 0;
}

int
main (void)
{
  if (test (1.0f) != 0)
    abort ();
  if (test (0.0f) != 1)
    abort ();
  if (test (-0.0f) != 1)
    abort ();
  return 0;
}
