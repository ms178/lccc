.text
.global _start
_start:
    xorl %ebp, %ebp
    andq $-16, %rsp
    call test
    movl %eax, %edi
    movl $60, %eax
    syscall
.section .note.GNU-stack,"",@progbits
