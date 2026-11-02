/* Standalone GNU C fixture, never a kernel compilation. */
#define FC __attribute__((ms_abi,noinline))
typedef int __attribute__((ms_abi)) Fn(int,int,int);
static __attribute__((noinline)) int outer(int seed)
{
    int base=seed;
    int FC add(int a,int b,int c) { return base+a+2*b+3*c; }
    int FC sibling(int a,int b,int c) { return add(a,b,c)+1; }
    int FC child(int a,int b,int c) {
        int saved=a+b+c;
        int FC inner(int x,int y,int z) { return base+saved+x+2*y+3*z; }
        Fn *volatile p=inner;
        return p(1,2,3);
    }
    Fn *volatile p=add;
    Fn *volatile q=sibling;
    for (int i=0;i<31;++i) {
        if (add(1,2,3)!=seed+14 || p(1,2,3)!=seed+14 ||
            q(1,2,3)!=seed+15 || child(1,2,3)!=seed+20 ||
            sibling(add(1,0,0),2,3)!=2*seed+15)
            return 1;
        ++base;
        ++seed;
    }
    return 0;
}
int main(void) { return outer(7) || outer(29); }
