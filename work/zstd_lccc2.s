.section .rodata
.Lstr0:
    .byte 37, 48, 49, 54, 108, 108, 120, 10, 0

.section .bss
.align 16
.type buffer, @object
.size buffer, 1048640
buffer:
    .zero 1048640

.text
.p2align 4
.type zstd_count, @function
zstd_count:
.cfi_startproc
    pushq %rbx
    .cfi_def_cfa_offset 16
    # LCCC_RET_XMM 0
    movq %rdi, %rbx
    movq %rdx, %rdi
    subq $7, %rdi
    movq %rbx, %r8
.LBB1:
    cmpq %rdi, %r8
jb .LBB3
.LBB2:
    movq %r8, %r9
    movq %rsi, %r11
    jmp .LBB6
.p2align 4,,10
.p2align 3
.LBB3:
    movq (%rsi), %r10
    xorq (%r8), %r10
jne .LBB5
.LBB4:
    addq $8, %r8
    addq $8, %rsi
    cmpq %rdi, %r8
jb .LBB3
    jmp .LBB2
.LBB5:
    movq %r10, %rax
    testq %r10, %r10
    jnz .Lctz_nz_0
    movl $64, %eax
    jmp .Lctz_done_1
.Lctz_nz_0:
    bsfq %rax, %rax
.Lctz_done_1:
    sarl $3, %eax
    cltq
    movslq %eax, %r10
    addq %r10, %r8
    subq %rbx, %r8
    movl %r8d, %eax
    popq %rbx
    ret
.LBB6:
    cmpq %rdx, %r9
jae .LBB9
.p2align 4,,10
.p2align 3
.LBB7:
    movzbl (%r9), %esi
    movzbl (%r11), %r8d
    cmpl %r8d, %esi
jne .LBB9
.LBB8:
    addq $1, %r9
    addq $1, %r11
    cmpq %rdx, %r9
jb .LBB7
.LBB9:
    subq %rbx, %r9
    movl %r9d, %eax
    popq %rbx
    ret
.cfi_endproc
.size zstd_count, .-zstd_count

.globl main
.p2align 4
.type main, @function
main:
.cfi_startproc
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    pushq %rbp
    subq $24, %rsp
    .cfi_def_cfa_offset 80
    # LCCC_RET_XMM 0
    leaq buffer(%rip), %rbx
    xorl %r12d, %r12d
    movl $2235986729, %r10d
.LBB11:
    cmpl $1048576, %r12d
jb .LBB13
.LBB12:
    xorl %r13d, %r13d
    xorl %r11d, %r11d
    movl $324508639, %r8d
    jmp .LBB17
.p2align 4,,10
.p2align 3
.LBB13:
    imull $1664525, %r10d, %r10d
    leal 1013904223(%r10), %r14d
    movl %r14d, %r9d
    andl $7, %r9d
    sete %dil
    movzbl %dil, %edi
    cmpl $64, %r12d
    setae %sil
    movzbl %sil, %esi
    andl %esi, %edi
je .LBB15
.LBB14:
    leal -64(%r12), %edx
    movl %r14d, %r10d
    shrl $28, %r10d
    addl %r10d, %edx
    movzbl (%rbx, %rdx), %r15d
    jmp .LBB16
.LBB15:
    movl %r14d, %r9d
    shrl $24, %r9d
    movzbl %r9b, %r15d
.LBB16:
    movl %r12d, %edi
    movq %r15, %rax
    movb %al, (%rbx, %rdi)
    addl $1, %r12d
    movq %r14, %r10
    cmpl $1048576, %r12d
jb .LBB13
    jmp .LBB12
.LBB17:
    cmpl $16, %r13d
jae .LBB22
.p2align 4,,10
.p2align 3
.LBB18:
    movq %r11, %rbp
    xorl %ebx, %ebx
    movq %r8, %r12
.LBB19:
    cmpl $131072, %ebx
jae .LBB21
.p2align 4,,10
.p2align 3
.LBB20:
    imull $1664525, %r12d, %edx
    leal 1013904223(%rdx), %r12d
    movl %r12d, %r15d
    andl $1048319, %r15d
    movl %r12d, %r10d
    shrl $12, %r10d
    andl $1048319, %r10d
    movl %r12d, %r8d
    andl $127, %r8d
    addl $16, %r8d
    movl %r15d, %r9d
    leaq buffer(%rip), %rdi
    addq %r9, %rdi
    movl %r10d, %esi
    leaq buffer(%rip), %r10
    addq %rsi, %r10
    movl %r8d, %edx
    leaq (%rdi, %rdx, 1), %r11
    movq %r10, %rsi
    movq %r11, %rdx
    call zstd_count
    movl %eax, %r9d
    movl %ebx, %edi
    andl $7, %edi
    shll $3, %edi
    movl %edi, %ecx
    shlq %cl, %r9
    movl %r15d, %esi
    xorq %rsi, %r9
    addq %r9, %rbp
    addl $1, %ebx
    cmpl $131072, %ebx
jb .LBB20
.LBB21:
    addl $1, %r13d
    movq %rbp, %r11
    movq %r12, %r8
    cmpl $16, %r13d
jb .LBB18
.LBB22:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    xorl %eax, %eax
    addq $24, %rsp
    popq %rbp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    ret
.cfi_endproc
.size main, .-main


.section .note.GNU-stack,"",@progbits
