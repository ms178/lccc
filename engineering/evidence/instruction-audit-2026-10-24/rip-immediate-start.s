.text
.globl _start
_start:
    subq $32, %rsp
    movq %rsp, %rdi
    call shuffle_symbolic_load
    cmpl $1, (%rsp)
    jne .Lfail
    cmpl $1, 12(%rsp)
    jne .Lfail
    movq %rsp, %rdi
    call shift_symbolic_load
    cmpl $8, (%rsp)
    jne .Lfail
    cmpl $0xfffffff8, 4(%rsp)
    jne .Lfail
    cmpl $0, 8(%rsp)
    jne .Lfail
    cmpl $0x91a2b3c0, 12(%rsp)
    jne .Lfail
    call extract_symbolic_store
    cmpl $1, rip_output(%rip)
    jne .Lfail
    cmpl $1, rip_output+12(%rip)
    jne .Lfail
    cmpb $0xa5, rip_output+16(%rip)
    jne .Lfail
    xorl %edi, %edi
    jmp .Lexit
.Lfail:
    movl $1, %edi
.Lexit:
    movl $60, %eax
    syscall
.section .note.GNU-stack,"",@progbits
