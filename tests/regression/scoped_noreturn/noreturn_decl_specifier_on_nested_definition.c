/* `_Noreturn` written on the nested definition is a contract about *it*: the call
 * cannot fall through, so no warning is owed.  A nested parser that copies only
 * four attribute flags drops this one, which makes the warning (and the
 * fallthrough answer behind it) wrong while the attribute was read fine on a
 * file-scope definition. */
int g(void) { _Noreturn auto void n(void) { for (;;) {} } n(); return 0; }
