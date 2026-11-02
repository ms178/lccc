.Lstr0:

main:
    subq $8, %rsp
    leaq .Lstr0(%rip), %rdi
    movl $16381, %esi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $8, %rsp
    ret


