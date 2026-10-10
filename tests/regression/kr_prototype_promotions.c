/* K&R definition metadata is not a call prototype. Body types must still
   survive in sema for ABI default promotions and parameter narrowing. */
__attribute__((noinline)) static double old(x, y, z)
float x;
signed char y;
unsigned short z;
{
    return x + y + z;
}

/* A visible prototype remains a prototype even when the definition is K&R. */
static double prior(double);
__attribute__((noinline)) static double prior(x) float x; { return x; }
__attribute__((noinline)) static double later(x) float x; { return x; }
static double later(double);

/* Prevent interprocedural constant folding from hiding a caller/callee ABI
   mismatch. This exposed the pre-existing later-prototype F32/F64 miscompile. */
static volatile float input = 1.25f;

int main(void)
{
    float x = input;
    signed char y = -12;
    unsigned short z = 60000;
    double (*indirect)() = old;
    if (old(x, y, z) != 59989.25 || indirect(x, y, z) != 59989.25)
        return 1;
    if (prior(x) != 1.25 || later(x) != 1.25)
        return 2;
    return 0;
}
