/* GCC 14 rejects this spelling on a definition ("attributes should be specified
 * before the declarator in a function definition"), so there is no oracle here:
 * lccc accepts the attribute and honours it, which is a deliberate superset --
 * the alternative is to discard what the user wrote.  No warning is owed. */
extern void die(void);
int g(void) { auto void die(void) __attribute__((noreturn)) { for (;;) {} } die(); }
