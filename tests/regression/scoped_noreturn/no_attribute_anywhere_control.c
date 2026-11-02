/* The control row: nothing is noreturn here, so the warning is owed by anyone.
 * A gate without a row like this passes a compiler whose diagnostics are off. */
extern void die(void);
int g(void) { auto void die(void) { return; } die(); }
