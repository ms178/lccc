// Regression pin (all opt levels): constant-condition terminators must fold
// BEFORE unreachable-block elimination, and dead-region elimination must keep
// exactly the labels that are transitively reachable from the entry CFG
// (case-dispatch and goto targets, plus their fall-through tails).
//
// GCC front-end parity oracle (host gcc -O0, undefined symbols at link):
//   * unlabeled statements in a never-executed branch die (medce-1.c,
//     gcc -O0 keeps `link_error` unreferenced);
//   * a case label inside a dead region survives WITH its body when the
//     enclosing switch can dispatch to it (medce-1.c shape, foo(1) runs
//     `case 1: bar();`);
//   * a goto-reachable label inside a dead region survives with its body;
//   * an UNREFERENCED plain label inside a dead region dies with its body
//     (dce1.c g3 oracle: gcc -O0 emits no call);
//   * switch on a constant folds to the matching case / default.
// If any rule regresses, the link fails on link_error_N or the run aborts.
extern void abort (void);
extern void link_error_case (void);
extern void link_error_goto (void);
extern void link_error_unref (void);
extern void link_error_switch (void);
extern void link_error_while (void);

int ok_case;
int ok_goto;
int ok_switch;

/* medce-1 interlock: `case 1` sits inside the dead `if (0)` region. The
   unlabeled call before it must die, the labeled body must survive. */
void
case_in_dead_if (int x)
{
  switch (x)
    {
    case 0:
      break;
    case 1:
      if (0)
	{
	  link_error_case ();
	case_in:
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
  case_in_dead_if (1);
  goto_into_dead_if (1);
  unref_label_dies (0);
  const_switch_folds ();
  while_zero_dies ();
  if (ok_case != 1 || ok_goto != 1 || ok_switch != 1)
    abort ();
  return 0;
}
