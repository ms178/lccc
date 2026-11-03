/* NON-KERNEL dynamic realignment/spilled-target ABI contract. */
typedef int V __attribute__((vector_size(32)));
typedef int __attribute__((fastcall)) F(int,V,V,V,V,int,int);
extern int __attribute__((fastcall)) vector_entry(int,V,V,V,V,int,int);
__asm__(".text\n.globl vector_entry\nvector_entry:\n"
        "leal 4(%esp),%eax\n andl $31,%eax\n jnz 9f\n"
        "vmovd %xmm0,%eax\n cmpl $1,%eax\n jne 9f\n"
        "vmovd %xmm1,%eax\n cmpl $1,%eax\n jne 9f\n"
        "vmovd %xmm2,%eax\n cmpl $1,%eax\n jne 9f\n"
        "movl 32(%esp),%eax\n addl %ecx,%eax\n"
        "leal (%eax,%edx,2),%eax\n movl 36(%esp),%ecx\n"
        "leal (%ecx,%ecx,2),%ecx\n addl %ecx,%eax\n ret $36\n"
        "9: movl $-999,%eax\n ret $36\n");
F *volatile target=vector_entry;
int main(void) {
    V v={1,2,3,4,5,6,7,8};
    for(int i=0;i<127;++i) if(target(1,v,v,v,v,2,3)!=22) return 1;
    return 0;
}
