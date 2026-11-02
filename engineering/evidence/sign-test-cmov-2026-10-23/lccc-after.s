.section .rodata
.Lstr0:
    .byte 37, 108, 108, 117, 10, 0

.section .rodata
.align 16
.type main.values.0, @object
.size main.values.0, 28
main.values.0:
    .long 2147483648
    .long 2147483649
    .long 4294967295
    .zero 4
    .long 1
    .long 2147483646
    .long 2147483647

.text
.globl nonnegative_difference
.p2align 4
.type nonnegative_difference, @function
nonnegative_difference:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    subl %esi, %edi
    movl $0, %ecx
    movl %ecx, %eax
    cmovnsl %edi, %eax
    ret
.cfi_endproc
.size nonnegative_difference, .-nonnegative_difference

.globl negative_difference
.p2align 4
.type negative_difference, @function
negative_difference:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    subl %esi, %edi
    movl $0, %eax
    cmovsl %edi, %eax
    ret
.cfi_endproc
.size negative_difference, .-negative_difference

.globl positive_difference
.p2align 4
.type positive_difference, @function
positive_difference:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    subl %esi, %edi
    testl %edi, %edi
    movl $0, %eax
    cmovgl %edi, %eax
    ret
.cfi_endproc
.size positive_difference, .-positive_difference

.globl main
.p2align 4
.type main, @function
main:
.cfi_startproc
    pushq %rbx
    .cfi_def_cfa_offset 16
    .cfi_offset %rbx, -16
    pushq %r12
    .cfi_def_cfa_offset 24
    .cfi_offset %r12, -24
    pushq %r13
    .cfi_def_cfa_offset 32
    .cfi_offset %r13, -32
    pushq %r14
    .cfi_def_cfa_offset 40
    .cfi_offset %r14, -40
    pushq %r15
    .cfi_def_cfa_offset 48
    .cfi_offset %r15, -48
    pushq %rbp
    .cfi_def_cfa_offset 56
    .cfi_offset %rbp, -56
    subq $24, %rsp
    .cfi_def_cfa_offset 80
    # LCCC_RET_XMM 0
    # LCCC_RET_XMM_BITS 512
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    leaq main.values.0(%rip), %rbx
    xorl %r11d, %r11d
    movq $0, 16(%rsp)
.LBB4:
    movl $0, %eax
    cmpq $7, %rax
jae .LBB9
.p2align 4,,10
.p2align 3
.LBB5:
    movq 16(%rsp), %rcx
    shlq $2, %rcx
    movq %rcx, 8(%rsp)
    leaq main.values.0(%rip), %r14
    addq %rcx, %r14
    movq %r11, %r15
    xorl %ebp, %ebp
.LBB6:
    cmpq $7, %rbp
jae .LBB8
.p2align 4,,10
.p2align 3
.LBB7:
    movl (%r14), %r13d
    movl (%rbx, %rbp, 4), %r12d
    imulq $33, %r15, %rax
    movq %rax, 8(%rsp)
    movl %r13d, %edi
    movl %r12d, %esi
    call nonnegative_difference@PLT
    # LCCC_CALL_ARGS 2
    # LCCC_CALL_FP 0
    movl %eax, %edi
    movq 8(%rsp), %rax
    addq %rdi, %rax
    imulq $33, %rax, %rax
    movq %rax, 8(%rsp)
    movl %r13d, %edi
    movl %r12d, %esi
    call negative_difference@PLT
    # LCCC_CALL_ARGS 2
    # LCCC_CALL_FP 0
    movl %eax, %r10d
    movq 8(%rsp), %r9
    addq %r10, %r9
    imulq $33, %r9, %rax
    movq %rax, 8(%rsp)
    movl %r13d, %edi
    movl %r12d, %esi
    call positive_difference@PLT
    # LCCC_CALL_ARGS 2
    # LCCC_CALL_FP 0
    movl %eax, %edi
    movq 8(%rsp), %r15
    addq %rdi, %r15
    addq $1, %rbp
    cmpq $7, %rbp
jb .LBB7
.LBB8:
    addq $1, 16(%rsp)
    movq %r15, %r11
    movq 16(%rsp), %rax
    cmpq $7, %rax
jb .LBB5
.LBB9:
    leaq .Lstr0(%rip), %rdi
    movq %r11, %rsi
    xorl %eax, %eax
    call printf@PLT
    # LCCC_VA_CALL
    # LCCC_CALL_ARGS 2
    # LCCC_CALL_FP 0
    xorl %eax, %eax
    addq $24, %rsp
    .cfi_def_cfa_offset 56
    popq %rbp
    .cfi_def_cfa_offset 48
    popq %r15
    .cfi_def_cfa_offset 40
    popq %r14
    .cfi_def_cfa_offset 32
    popq %r13
    .cfi_def_cfa_offset 24
    popq %r12
    .cfi_def_cfa_offset 16
    popq %rbx
    .cfi_def_cfa_offset 8
    ret
.cfi_endproc
.size main, .-main


.section .note.GNU-stack,"",@progbits
