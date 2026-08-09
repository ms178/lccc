extern int foo(int x) __asm__("bar");
int foo(int x) { return x * 2 + 1; }
int main(void) { return foo(21) == 43 ? 0 : 1; }
