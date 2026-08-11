.section .rodata
.Lstr0:
    .byte 37, 100, 32, 37, 100, 10, 0

.section .text
.globl main
.type main, @function
main:
.cfi_startproc
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $264, %rsp
    .cfi_def_cfa_offset 320
    movl $1, 216(%rsp)
    movl $2, 220(%rsp)
    movl $3, 224(%rsp)
    movl $4, 228(%rsp)
    movl $5, 232(%rsp)
    movl $6, 236(%rsp)
    movl $7, 240(%rsp)
    movl $8, 244(%rsp)
    movl $9, 248(%rsp)
    leaq .Lstr0(%rip), %r14
    vpxor %ymm0, %ymm0, %ymm0
    leaq 184(%rsp), %rdx
    vmovdqu %ymm0, (%rdx)
    vpxor %ymm0, %ymm0, %ymm0
    vmovupd 184(%rsp), %ymm0
    vmovupd %ymm0, 152(%rsp)
    xorl %r13d, %r13d
.LBB1:
    cmpl $1, %r13d
jge .LBB1
.LBB2:
    movslq %r13d, %r8
    movq %r8, %r9
    shlq $5, %r9
    leaq 216(%rsp), %rcx
    movq %r9, %rax
    addq %rcx, %rax
    movq %rax, %rdi
    xorl %ecx, %ecx
    vmovdqu (%rax,%rcx), %ymm0
    leaq 120(%rsp), %rdx
    vmovdqu %ymm0, (%rdx)
    leaq 152(%rsp), %rax
    vmovdqu (%rax), %ymm0
    leaq 120(%rsp), %rcx
    leaq 120(%rsp), %rcx
    vmovdqu (%rcx), %ymm1
    vpaddd %ymm1, %ymm0, %ymm0
    leaq 88(%rsp), %rdx
    vmovdqu %ymm0, (%rdx)
    movq %r13, %rax
    addl $1, %eax
    cltq
    movq %rax, 80(%rsp)
    jmp .LBB1
.LBB3:
    testl %esi, %esi
jge .LBB3
.LBB4:
    movslq %esi, %r11
    movq %r11, %r10
    shlq $5, %r10
    movq %r10, %rax
    addq %rcx, %rax
    movq %rax, %r8
    xorl %ecx, %ecx
    vmovdqu (%rax,%rcx), %ymm0
    leaq 48(%rsp), %rdx
    vmovdqu %ymm0, (%rdx)
    leaq 16(%rsp), %rax
    vmovdqu (%rax), %ymm0
    leaq 48(%rsp), %rcx
    leaq 48(%rsp), %rcx
    vmovdqu (%rcx), %ymm1
    vpaddd %ymm1, %ymm0, %ymm0
    leaq 16(%rsp), %rdx
    vmovdqu %ymm0, (%rdx)
    addl $4, %esi
    jmp .LBB3
.LBB5:
    movq %r14, %rdi
    leaq 152(%rsp), %rsi
    movq %r12, %rdx
    xorl %eax, %eax
    call printf
    xorl %eax, %eax
    addq $264, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.LBB1:
    vmovdqu 152(%rsp), %ymm0
    vextracti128 $1, %ymm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $8, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vpsrldq $4, %xmm0, %xmm1
    vpaddd %xmm1, %xmm0, %xmm0
    vmovd %xmm0, %eax
    movq %rax, 8(%rsp)
    movl %r13d, %r9d
    shll $3, %r9d
    movq %r9, %r15
    movq %rax, %rbp
.LBB2:
    cmpl $9, %r15d
jge .LBB3
.LBB6:
    movslq %r15d, %rsi
    movq %rsi, %rdx
    shlq $2, %rdx
    leaq 216(%rsp), %rcx
    movl (%rcx, %rdx), %r10d
    addl %r10d, %ebp
    addl $1, %r15d
    jmp .LBB2
.LBB3:
    movslq %ebx, %r8
    movq %r8, %r9
    shlq $2, %r9
    movl (%rcx, %r9), %esi
    addl %esi, %r12d
.LBB4:
    addl $4, %ebx
    cmpl $5, %ebx
jl .LBB3
    jmp .LBB5
.LBB7:
    vmovupd 88(%rsp), %ymm0
    vmovupd %ymm0, 152(%rsp)
    movq 80(%rsp), %r13
    jmp .LBB1
.cfi_endproc
.size main, .-main


.section .note.GNU-stack,"",@progbits
