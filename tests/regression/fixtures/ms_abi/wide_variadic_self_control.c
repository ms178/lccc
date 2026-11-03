/* Non-kernel oracle suitability control, NOT an LCCC verdict.
 * GCC 16.2.0: exit 1; Clang 23.1.4: exit 0 on x86-64 Linux.
 * The GCC caller supplies indirect integer128 images but its va_arg consumes
 * aligned inline bytes. Use a self-checked Clang receiver in check_ms_abi.sh.
 * Reproduce: gcc-16 -O2 this-file.c -o control && ./control
 */
typedef unsigned __int128 U;
__attribute__((ms_abi,noinline)) U receive(int n,...) {
    __builtin_ms_va_list ap;
    __builtin_ms_va_start(ap,n);
    U sum=0;
    while(n--)sum+=__builtin_va_arg(ap,U);
    __builtin_ms_va_end(ap);
    return sum;
}
int main(void) { return receive(4,(U)1,(U)2,(U)3,(U)4)!=10; }
