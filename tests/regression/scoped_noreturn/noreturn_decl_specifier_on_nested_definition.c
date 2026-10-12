/* `_Noreturn` written on the nested definition is a contract about *it*: the call
 * cannot fall through, so no warning is owed.  The nested parser used to copy
 * four attribute flags and drop this one, which made the warning (and the
 * fallthrough answer behind it) wrong while the attribute was read fine on a
 * file-scope definition. */
int g(void) { _Noreturn auto void n(void) { for (;;) {} } n(); return 0; }
