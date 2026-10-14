.section .bss
.align 16
.type guard, @object
.size guard, 256
guard:
    .zero 256
.align 16
.type tree, @object
.size tree, 576
tree:
    .zero 576

.text
.globl main
.p2align 4
.type main, @function
main:
.cfi_startproc
    subq $88, %rsp
    .cfi_def_cfa_offset 96
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    leaq guard(%rip), %r11
    xorl %r10d, %r10d
.LBB1:
    cmpq $256, %r10
jge .LBB3
.p2align 4,,10
.p2align 3
.LBB2:
    movb %r10b, (%r11, %r10)
    addq $1, %r10
    cmpq $256, %r10
jl .LBB2
    jmp .LBB3
.LBB4:
    cmpq $144, %rdi
jge .LBB6
.p2align 4,,10
.p2align 3
.LBB5:
    movw $8, (%rdx, %rdi, 2)
    addw $1, (%rsi)
    addq $1, %rdi
    cmpq $144, %rdi
jl .LBB5
.LBB6:
    leaq 56(%rsp), %rdi
    leaq guard(%rip), %rsi
    leaq tree(%rip), %rdx
    call check@PLT
    # LCCC_CALL_ARGS 3
    # LCCC_CALL_FP 0
    movl %eax, %eax
    addq $88, %rsp
    .cfi_def_cfa_offset 8
    ret
.LBB3:
    .cfi_def_cfa_offset 96
    vpxor %ymm0, %ymm0, %ymm0
    vmovdqu %ymm0, 56(%rsp)
    leaq tree(%rip), %rdx
    xorl %edi, %edi
    jmp .LBB4
.cfi_endproc
.size main, .-main


.section .note.GNU-stack,"",@progbits
