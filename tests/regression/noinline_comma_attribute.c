/* The attribute belongs to the second declarator, not its sibling. The gate
 * checks caller's assembly as well as execution; value equality alone cannot
 * detect an accidentally inlined definition. */
int sibling(int), kept(int) __attribute__((noinline));
int kept(int x) { return x * 17 + 3; }
int sibling(int x) { return x * 5 - 2; }
__attribute__((noinline)) int caller(int x) { return kept(x) + sibling(x); }
volatile int input = 9;
int main(void) { int x = input; return caller(x) != x * 22 + 1; }
