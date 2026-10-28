volatile int source;
volatile int *p;
int *volatile vp;
__typeof__(*p) from_deref;
__typeof__(vp) ptr_copy;
typedef __typeof__(source) VI;
VI from_alias;
void f(void) { from_deref; from_deref; }
void g(void) { ptr_copy; ptr_copy; }
void h(void) { from_alias; from_alias; }
