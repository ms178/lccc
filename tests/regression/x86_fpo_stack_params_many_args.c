/* gcc.c-torture/execute/20010518-1.c reduced.
 * FPO leaf functions that save callee-saved register homes with push/pop still
 * must address stack-passed arguments relative to the live %rsp below those
 * pushes. The 7th+ args used to reload saved registers instead.
 */
extern void abort(void);
int add13(int a,int b,int c,int d,int e,int f,int g,int h,int i,int j,int k,int l,int m) {
    return a+b+c+d+e+f+g+h+i+j+k+l+m;
}
int main(void) {
    if (add13(1,2,3,4,5,6,7,8,9,10,11,12,13) != 91) abort();
    return 0;
}
