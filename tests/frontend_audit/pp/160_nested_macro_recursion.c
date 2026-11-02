#define A(x) B(x)
#define B(x) A(x)
A(1)
#define L1 L2
#define L2 L3
#define L3 done
L1
#define WRAP(x) [x]
#define ID(x) x
WRAP(ID(WRAP(1)))
