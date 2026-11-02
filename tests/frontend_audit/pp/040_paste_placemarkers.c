#define CAT(a,b) a ## b
#define XCAT(a,b) CAT(a,b)
#define EM
a: CAT(,) ;
b: CAT(1,) ; CAT(,2) ; CAT(3,4) ;
c: XCAT(EM,EM) x ;
d: CAT(L, "wide") ;
e: CAT(u8, "utf") ;
f: CAT(+, +) x ;
g: 1 CAT(e, +3) ;
#define PREFIX(fn) my_ ## fn
PREFIX(foo)(1);
