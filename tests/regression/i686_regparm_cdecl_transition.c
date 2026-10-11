/* A call's zero-register convention must replace the caller's regparm(3).
   Also run with -mregparm={0,1,2,3}: an explicit regparm(0) callee is always
   cdecl, while an unannotated callee uses the TU default, not the caller. */
#define NI __attribute__((noinline))
static NI __attribute__((regparm(0))) unsigned cdecl_zero(unsigned a, unsigned b, unsigned c)
{
    return a * 3U + b * 5U + c * 7U;
}
static NI unsigned ordinary(unsigned a, unsigned b, unsigned c)
{
    return a * 3U + b * 5U + c * 7U;
}
static NI __attribute__((regparm(3))) unsigned caller(unsigned a)
{
    unsigned x = cdecl_zero(a, 19, 23);
    return x + ordinary(a, 29, 31);
}
static volatile unsigned input = 17;
int main(void)
{
    return caller(input) != 720;
}
