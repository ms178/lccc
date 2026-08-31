/* S11 regression pin (O1/O2): two interacting hazards on the indexed-gep
 * fold path.
 *
 * 1. The fold's SIB index can be a const-initialized I32 copy chain
 *    (`posGreatest = -1;` then phi/threaded copies). Copy-from-constant
 *    defs carry no value_types entry unless the builder seeds them from
 *    the constant's own variant; an untyped index must not be gambled on.
 *
 * 2. When the emitter refuses a fold, rematerialising the skipped GEP
 *    rebuilds the address through %rax and must not destroy an
 *    accumulator-resident store value whose producing load was
 *    dead-producer-skipped (its only copy lives in %rax, no home/slot) —
 *    the acc is protected around the remat (gcc.c-torture 20020402-1).
 *
 * Derived from PR c/2100; expected exit 0. */

extern void abort (void);
extern void exit (int);

#define SMALL_N  2
#define NUM_ELEM 4

int
main (void)
{
  int listElem[NUM_ELEM] = { 30, 2, 10, 5 };
  int listSmall[SMALL_N];
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

  if (listSmall[0] != 5 || listSmall[1] != 10)
    abort ();

  exit (0);
}
