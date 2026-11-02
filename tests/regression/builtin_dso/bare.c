/* -shared -nostartfiles (check_builtin_dso_link.sh): no crt objects, so no
   _init -- and a .init fragment with no crti prologue/ret must get no
   DT_INIT (GNU ld emits none; the loader would run off its end). */
__asm__(".section .init,\"ax\"\n nop\n.text\n");
int bare_value(int x) { return x * 3; }
