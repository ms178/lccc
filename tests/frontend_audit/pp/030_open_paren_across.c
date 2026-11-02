/* A replacement list that opens a call completed by later source tokens. */
#define x 2
#define f(a) f(x * (a))
#define g f
#define h g(~
h 5);
#define only_call (
#define twice(a) a + a
only_call twice(3))
