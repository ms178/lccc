#define ADD(a,b) ((a)+(b))
ADD(1,
    2)
#define MANY(x) x
MANY(
1
+
2
)
#define NEST(a,b) ADD(ADD(a,b),ADD(b,a))
NEST(1,
  2
  )
#define APPLY(f, a) f(a)
APPLY(MANY, 3)
