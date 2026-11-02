#define f(a) a*g
#define g(a) f(a)
f(2)(9);
#define AA BB
#define BB AA
AA BB
#define sq(x) (x)*(x)
#define apply(m, a) m(a)
apply(apply(sq, 3) + 1, 2);
#define rec(x) rec(x) + 1
rec(1);
#define self self + 1
self
#define e1 e2
#define e2 e1
e1 e2
