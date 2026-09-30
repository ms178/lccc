.section .bss
.align 16
.type v, @object
.size v, 2048
v:
    .zero 2048

.text
.globl bench_setup
.p2align 4
.type bench_setup, @function
bench_setup:
.cfi_startproc
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 0
    leaq v(%rip), %rdi
    xorl %esi, %esi
.LBB1:
    cmpl $2048, %esi
jge .LBB3
.p2align 4,,10
.p2align 3
.LBB2:
    movslq %esi, %rdx
    imull $37, %esi, %r9d
    movl %esi, %r11d
    andl $3, %r11d
    movl $128, %ecx
    movl $0, %r10d
    cmovnel %ecx, %r10d
    orl %r10d, %r9d
    movb %r9b, (%rdi, %rdx)
    addl $1, %esi
    cmpl $2048, %esi
jl .LBB2
.LBB3:
    ret
.cfi_endproc
.size bench_setup, .-bench_setup

.globl bench_run
.p2align 4
.type bench_run, @function
bench_run:
.cfi_startproc
    pushq %rbx
    .cfi_def_cfa_offset 16
    .cfi_offset %rbx, -16
    pushq %r12
    .cfi_def_cfa_offset 24
    .cfi_offset %r12, -24
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorl %edi, %edi
    xorl %esi, %esi
.LBB5:
    leal 4(%rsi), %edx
    cmpl $2048, %edx
jge .LBB12
.p2align 4,,10
.p2align 3
.LBB6:
    movslq %esi, %r8
    leaq v(%rip), %r9
    addq %r8, %r9
    movzbl (%r9), %r11d
    movl %r11d, %r10d
    cmpb $-128, %r11b
jae .LBB8
.LBB7:
    movq %r10, %rdx
    movl $1, %ebx
    jmp .LBB11
.LBB8:
    andl $127, %r10d
    movl %r10d, %r11d
    shll $7, %r11d
    movzbl 1(%r9), %edx
    movl %edx, %r8d
    cmpb $-128, %dl
jae .LBB10
.LBB9:
    orl %r11d, %r8d
    movq %r8, %rdx
    movl $2, %ebx
    jmp .LBB11
.LBB10:
    andl $127, %r8d
    orl %r8d, %r11d
    shll $7, %r11d
    movzbl 2(%r9), %r10d
    movl %r10d, %r8d
    movl %r11d, %r12d
    orl %r10d, %r12d
    andl $127, %r8d
    orl %r8d, %r11d
    cmpb $-128, %r10b
    movl %r11d, %edx
    cmovbl %r12d, %edx
    movl $3, %ecx
    movl $4, %ebx
    cmovbl %ecx, %ebx
.LBB11:
    leal (%rbx, %rdx), %eax
    addq %rax, %rdi
    addl $1, %esi
    leal 4(%rsi), %edx
    cmpl $2048, %edx
jl .LBB6
.LBB12:
    movq %rdi, %rax
    popq %r12
    .cfi_def_cfa_offset 16
    popq %rbx
    .cfi_def_cfa_offset 8
    ret
.cfi_endproc
.size bench_run, .-bench_run


.section .note.GNU-stack,"",@progbits
