/* S11 regression pin (O1/O2): indexed-gep fold hazards in the x86 backend.
 *
 * 1. The fold's SIB index can be a const-initialized I32 copy chain
 *    (`posGreatest = -1;` then phi/threaded copies). Copy-from-constant
 *    defs carry no value_types entry unless the builder seeds them from
 *    the constant's own variant; an untyped index must not be gambled on.
 *
 * 2. When the emitter refuses a fold, rematerialising the skipped GEP
 *    rebuilds the address through %rax and must not destroy an
 *    accumulator-resident store value whose producing load was
 *    dead-producer-skipped (its only copy lives in %rax, no home/slot).
 *
 * Historical note: the original pin declared listSmall[SMALL_N] (2
 * elements) while the algorithm writes listSmall[posGreatest] with
 * posGreatest reaching SMALL_N..NUM_ELEM-1, i.e. it performed an
 * out-of-bounds store whose effect no conforming compiler is obliged to
 * honor. That made the final assertion unsatisfiable by correct code:
 * GCC -O0..-O3 and LCCC -O0..-Os all abort() it (rc=134). A UB pin has no
 * signal. This version widens the window to NUM_ELEM so every store stays
 * in-bounds while preserving the const-initialized index chain, the
 * indexed-GEP store through listSmall[posGreatest], and the two-phase
 * greatest-replacement structure; the assertion checks the algorithm's
 * true result. Expected exit 0.
 */
extern void abort (void);
extern void exit (int);

#define SMALL_N  2
#define NUM_ELEM 4

int
main (void)
{
  int listElem[NUM_ELEM] = { 30, 2, 10, 5 };
  int listSmall[NUM_ELEM];
  int i, j;
  int posGreatest = -1, greatest = -1;

  for (i = 0; i < SMALL_N; i++)
    {
      listSmall[i] = listElem[i];
      if (listElem[i] > greatest)
	{
	  posGreatest = i;
	  greatest = listElem[i];
	}
    }

  for (i = SMALL_N; i < NUM_ELEM; i++)
    {
      if (listElem[i] < greatest)
	{
	  listSmall[posGreatest] = listElem[i];
	  posGreatest = i;
	  greatest = listElem[i];
	}
    }

  /* Trace: i=2 writes listSmall[0]=10 (pos 0->2), i=3 writes
     listSmall[2]=5 (pos 2->3); listSmall[1] keeps its 2 from phase one.  */
  if (listSmall[0] != 10 || listSmall[1] != 2 || listSmall[2] != 5)
    abort ();

  exit (0);
}
