// Regression pin (all opt levels): constant-condition terminators must fold
// BEFORE unreachable-block elimination, and dead-region elimination must keep
// exactly the labels that are transitively reachable from the entry CFG
// (case-dispatch and goto targets, plus their fall-through tails).
//
// GCC front-end parity oracle (host gcc -O0, undefined symbols at link):
//   * unlabeled statements in a never-executed branch die (medce-1.c,
//     gcc -O0 keeps `link_error` unreferenced);
//   * a case label inside a dead region survives WITH its body when the
//     enclosing switch can dispatch to it (medce-1.c shape, foo(2) runs
//     `case 2:` nested in the `if (0)` and lands on ok_case = 1);
//   * a goto-reachable label inside a dead region survives with its body;
//   * an UNREFERENCED plain label inside a dead region dies with its body
//     (dce1.c g3 oracle: gcc -O0 emits no call);
//   * switch on a constant folds to the matching case / default.
// If any rule regresses, the link fails on link_error_N or the run aborts.
//
// Historical note: the original pin put a plain (non-dispatchable) label
// `case_in:` inside the `if (0)` and then called foo(1), which only ever
// dispatches to the outer `case 1` before the dead region. Nothing can reach
// the inner label, so ok_case was never set and every conforming compiler --
// GCC -O0..-O3 and LCCC -O0..-Os -- aborted (rc=134). A dead pin has no
// signal. This version nests a dispatchable `case 2:` inside the `if (0)`
// (labels may sit anywhere in the switch body; dispatch jumps past the dead
// condition) and calls foo(2), which is exactly the medce-1 interlock the
// comment describes.
extern void abort (void);
extern void link_error_case (void);
extern void link_error_goto (void);
extern void link_error_unref (void);
extern void link_error_switch (void);
extern void link_error_while (void);

int ok_case;
int ok_goto;
int ok_switch;

/* medce-1 interlock: `case 2` sits inside the dead `if (0)` region under
   `case 1`. The unlabeled call before it must die, the labeled body must
   survive because switch dispatch (x == 2) can jump straight into it. */
void
case_in_dead_if (int x)
{
  switch (x)
    {
    case 1:
      if (0)
	{
	  link_error_case ();
	case 2:
	  ok_case = 1;
	}
      break;
    default:
      break;
    }
}

/* goto INTO a dead region: the target label's body survives. */
void
goto_into_dead_if (int x)
{
  if (0)
    {
    dead_goto_target:
      ok_goto = 1;
      return;
    }
  ok_goto = 2;
  if (x)
    goto dead_goto_target;
}

/* Unreferenced plain label in a dead region: dies with its body (dce1 g3). */
void
unref_label_dies (int x)
{
  if (0)
    {
      link_error_unref ();
    unreferenced_label:
      link_error_unref ();
    }
  (void) x;
}

/* switch on a constant folds to the matching case at every level. */
void
const_switch_folds (void)
{
  switch (3)
    {
    case 1:
      link_error_switch ();
      break;
    case 3:
      ok_switch = 1;
      break;
    case 5:
      link_error_switch ();
      break;
    default:
      link_error_switch ();
      break;
    }
}

/* while (0) bodies die at every level. */
void
while_zero_dies (void)
{
  while (0)
    {
      link_error_while ();
    }
}

int
main (void)
{
  case_in_dead_if (2);
  goto_into_dead_if (1);
  unref_label_dies (0);
  const_switch_folds ();
  while_zero_dies ();
  if (ok_case != 1 || ok_goto != 1 || ok_switch != 1)
    abort ();
  return 0;
}
