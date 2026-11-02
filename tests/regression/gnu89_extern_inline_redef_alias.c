/* GNU89 extern-inline + out-of-line redefinition + weak alias (glibc
 * libio/feof_u.c shape, LK-23).
 *
 * In gnu89 inline semantics (-fgnu89-inline or __attribute__((gnu_inline))),
 * an `extern __inline` body does NOT provide the external definition; GCC
 * explicitly permits a later plain definition of the same function in the
 * same TU, which becomes THE external definition and the authoritative body.
 *
 * Fixed defect: lower_function's first-wins duplicate guard silently dropped
 * the plain definition whenever the inline husk had been lowered first —
 * which happened exactly when something *referenced* the name, e.g. the
 * weak_alias declarator in feof_u.c. The object came out EMPTY (no
 * __feof_unlocked, no feof_unlocked) and glibc's ldconfig link died with
 * `undefined reference to feof_unlocked`.
 *
 * The bodies below intentionally DIFFER (inline: +1, plain: +2) so the test
 * proves the plain body is the one emitted AND the one the alias binds to —
 * a compiler that resurrects the husk or keeps first-wins fails the value
 * checks, and a compiler that drops the definition fails to link.
 */
typedef struct F { int _flags; } XFILE;

extern int feof_x (XFILE *__stream) __attribute__ ((__nothrow__));
extern __inline int
__attribute__ ((__nothrow__)) feof_x (XFILE *__stream)
{
  return __stream->_flags + 1;
}

int
feof_x (XFILE *fp)
{
  return fp->_flags + 2;
}
extern __typeof (feof_x) feof_x_alias __attribute__ ((weak, alias ("feof_x")));

/* Attribute flavour (works independently of -fgnu89-inline). */
extern __inline __attribute__ ((__gnu_inline__)) int
gi (int x)
{
  return x + 10;
}
int
gi (int x)
{
  return x + 20;
}
extern __typeof (gi) gi_alias __attribute__ ((weak, alias ("gi")));

int
main (void)
{
  XFILE f = { ._flags = 40 };
  int ok = 1;
  ok &= feof_x (&f) == 42;       /* plain body: 40 + 2 */
  ok &= feof_x_alias (&f) == 42; /* alias binds to the plain body */
  ok &= gi (1) == 21;            /* plain body: 1 + 20 */
  ok &= gi_alias (1) == 21;
  __builtin_printf ("gnu89-redef-alias:%s\n", ok ? "ok" : "MISMATCH");
  return ok ? 0 : 1;
}
