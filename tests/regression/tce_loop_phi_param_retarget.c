/* Tail-call elimination + pre-existing loop phi fed by a parameter (LK-27).
 *
 * Shape distilled from glibc's _dl_lookup_symbol_x: the function body
 * contains a loop whose header phi takes the PARAMETER value on the entry
 * edge (here: the djb2 hash loop over `s`), and the function TAIL-CALLS
 * itself (the retry path). TCE inserts a loop header with one phi per
 * parameter and renames parameter uses to those phis.
 *
 * Fixed defect: TCE renamed the VALUES in successor phis but left their
 * predecessor labels pointing at the (former) entry block, whose terminator
 * had just moved into the new loop header. The hash-loop phi then claimed
 * to receive the TCE phi's value "from entry" — a block the TCE phi does
 * not dominate — and phi elimination materialized the edge copy in the
 * entry block, READING THE TCE PHI'S REGISTER HOME BEFORE ITS FIRST
 * DEFINITION. In glibc every ld.so symbol lookup hashed a stale register;
 * here the hash of the first argument comes out garbage.
 *
 * The test drives enough distinct strings through both the entry path and
 * the retry (tail-call) path that a stale-register hash cannot accidentally
 * produce the right sum, and compares against GCC via stdout.
 */
extern int printf (const char *, ...);

static unsigned
lookup (const char *s, unsigned depth, unsigned salt)
{
  /* djb2 over s — loop phi on `s` and on `h`, both fed by params. */
  unsigned h = 5381u + salt;
  const unsigned char *p = (const unsigned char *) s;
  for (unsigned c = *p; c != 0; c = *++p)
    h = h * 33u + c;

  if (depth == 0)
    return h;

  /* Retry path: tail call with SHIFTED arguments so the TCE phis carry
     different values per iteration (s advances, salt mixes the hash). */
  return lookup (s + (h & 1), depth - 1, h ^ salt);
}

int
main (void)
{
  static const char *names[] = {
    "_rtld_global_ro", "__progname", "_dl_lookup_symbol_x",
    "x", "", "memcpy", "__libc_early_init",
  };
  unsigned long long acc = 0;
  for (unsigned i = 0; i < sizeof names / sizeof *names; i++)
    for (unsigned d = 0; d < 4; d++)
      acc += lookup (names[i], d, i * 0x9e3779b9u);
  printf ("tce-loop-phi:%llx\n", acc);
  return 0;
}
