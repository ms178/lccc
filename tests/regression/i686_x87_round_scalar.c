/* S12 regression pin (i686, all opt levels): simplify.rs rewrites
 * floor/ceil/trunc/rint/nearbyint/roundeven into RoundScalar* intrinsics
 * unconditionally (required for glibc self-hosting on x86-64), but the i686
 * backend had no arm — the `_ => {}` fallthrough silently dropped the
 * instruction and the result slot was read unwritten (gcc.c-torture
 * float-floor: (int)floor(d) != 1023 -> abort at every opt level).
 * Lowered on i686 via x87 `frndint` with a transient RC-field switch and a
 * full control-word restore. Expected exit 0. */

static double d = 1024.0 - 1.0 / 32768.0;

extern double floor (double);
extern void abort (void);

int
main (void)
{
  double df = floor (d);
  float f1 = (float) floor (d);
  if ((int) df != 1023 || (int) f1 != 1023)
    abort ();
  return 0;
}
