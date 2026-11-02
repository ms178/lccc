/*
 * Regression pin: nested short-circuit fold must not strand blocks.
 *
 * Derived from gcc.c-torture/execute/20000314-1.c. The frontend's
 * fold_comparison_pair ((x==0)||(x!=0) -> true) fires inside the recursive
 * lower_condition_branch of the OUTER `||`, which has already created and
 * lowered into its rhs_label block. The stranded block referenced a value
 * (the `winds` pointer load) whose definition was removed together with the
 * folded branch. At -O0 no cfg cleanup ran, the block reached the x86
 * backend, and the session-26 hard gate (correctly refusing to fabricate a
 * home-less operand) ICE'd the compile. The driver now strips unreachable
 * blocks before codegen at every opt level.
 *
 * Contract:
 *   1. compiles at -O0/-O1/-O2 on both backends (no ICE),
 *   2. `winds` stays a real runtime variable (while-loop shape preserved),
 *   3. the tautology `(winds==0)||(winds!=0)` short-circuits the dangerous
 *      `*(char *) winds` dereference, so the program exits 0, never aborts.
 */
void exit(int);
void abort(void);

int main(void)
{
  long winds = 0;

  while (winds != 0)
    {
      if (*(char *) winds)
	break;
    }

  if (winds == 0 || winds != 0 || *(char *) winds)
    exit (0);

  abort ();
}
