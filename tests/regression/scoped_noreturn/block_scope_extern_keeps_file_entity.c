/* C11 6.7.1p5: a block-scope function declaration *without* `auto` denotes the
 * externally visible entity, so the outer `noreturn` applies and nothing falls
 * through.  The scoped fix must not over-apply and treat this as a new entity. */
extern void die(void) __attribute__((noreturn));
int g(void) { extern void die(void); die(); }
