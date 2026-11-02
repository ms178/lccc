/* The other half of consumption: `m` follows a `_Noreturn` nested definition in
 * the same block, and an attribute left in the parser's accumulated state would
 * silently become `m`'s too -- which would swallow this warning.  Measured
 * against GCC, which reports the fallthrough. */
int g(void) { _Noreturn auto void n(void) { for (;;) {} } void m(void) { return; } m(); }
