.Lstr0:

A:
B:
C:

matmul:
    pushq %rbp
    movq %rsp, %rbp
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    subq $32, %rsp
    leaq A(%rip), %rbx
    xorl %r12d, %r12d
    leaq C(%rip), %r8
    movq %rbx, %r13
.LBB1:
    cmpq $256, %r12
jge .LBB12
.LBB2:
    movq %r12, %rdi
    shlq $11, %rdi
    leaq A(%rip), %rax
    addq %rdi, %rax
    movq %rax, -48(%rbp)
    xorl %esi, %esi
    leaq B(%rip), %rdi
    movq %r13, %r14
.LBB3:
    cmpq $256, %rsi
jge .LBB8
.LBB4:
    vbroadcastsd (%r14), %ymm1
    xorl %r11d, %r11d
.LBB5:
    cmpq $2048, %r11
jge .LBB9
.LBB6:
    vmovupd (%r8,%r11), %ymm0
    vfmadd231pd (%rdi,%r11), %ymm1, %ymm0
    vmovupd %ymm0, (%r8,%r11)
    vmovupd 32(%r8,%r11), %ymm0
    vfmadd231pd 32(%rdi,%r11), %ymm1, %ymm0
    vmovupd %ymm0, 32(%r8,%r11)
    vmovupd 64(%r8,%r11), %ymm0
    vfmadd231pd 64(%rdi,%r11), %ymm1, %ymm0
    vmovupd %ymm0, 64(%r8,%r11)
    vmovupd 96(%r8,%r11), %ymm0
    vfmadd231pd 96(%rdi,%r11), %ymm1, %ymm0
    vmovupd %ymm0, 96(%r8,%r11)
    addq $128, %r11
    cmpq $2048, %r11
jl .LBB6
    jmp .LBB9
.LBB7:
    addq $1, %rsi
    leaq 2048(%rdi), %rdi
    leaq 8(%r14), %r14
    cmpq $256, %rsi
jl .LBB4
.LBB8:
    addq $1, %r12
    leaq 2048(%r8), %r8
    leaq 2048(%r13), %r13
    cmpq $256, %r12
jl .LBB2
    jmp .LBB12
.LBB9:
    movq %r11, %rax
    shrl $3, %eax
    movslq %eax, %r11
    leaq 0(,%r11,8), %r9
    leaq (%r8, %r9), %r10
    leaq (%rdi, %r9), %rdx
    leaq 0(,%rsi,8), %rax
    movq %rax, -56(%rbp)
    movq -48(%rbp), %rcx
    addq %rcx, %rax
    movq %rax, %r9
.LBB10:
    cmpq $256, %r11
jge .LBB7
.LBB11:
    vmovsd (%r10), %xmm2
    vmovsd (%r9), %xmm3
    vfmadd231sd (%rdx), %xmm3, %xmm2
    vmovsd %xmm2, (%r10)
    addq $1, %r11
    leaq 8(%r10), %r10
    leaq 8(%rdx), %rdx
    cmpq $256, %r11
jl .LBB11
    jmp .LBB7
.LBB12:
    leaq -32(%rbp), %rsp
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    popq %rbp
    ret

main:
    subq $24, %rsp
    movsd .LCFP_0(%rip), %xmm2
    xorl %r11d, %r11d
    leaq B(%rip), %r10
    leaq A(%rip), %r8
.LBB14:
    cmpl $256, %r11d
jge .LBB19
.LBB15:
    xorl %r9d, %r9d
.LBB16:
    cmpl $256, %r9d
jge .LBB18
.LBB17:
    movl %r11d, %eax
    addl %r9d, %eax
    vcvtsi2sdl %eax, %xmm0, %xmm3
    vdivsd %xmm2, %xmm3, %xmm3
    movsd %xmm3, (%r8, %r9, 8)
    movl %r11d, %esi
    imull %r9d, %esi
    leal 1(%rsi), %eax
    vcvtsi2sdl %eax, %xmm0, %xmm4
    vdivsd %xmm2, %xmm4, %xmm4
    movsd %xmm4, (%r10, %r9, 8)
    addl $1, %r9d
    cmpl $256, %r9d
jl .LBB17
.LBB18:
    addl $1, %r11d
    leaq 2048(%r10), %r10
    leaq 2048(%r8), %r8
    cmpl $256, %r11d
jl .LBB15
.LBB19:
    call matmul@PLT
    movsd C+263168(%rip), %xmm5
    movsd %xmm5, 16(%rsp)
    movsd 16(%rsp), %xmm6
    leaq .Lstr0(%rip), %rdi
    movsd %xmm6, %xmm0
    movb $1, %al
    call printf@PLT
    xorl %eax, %eax
    addq $24, %rsp
    ret

.LCFP_0:
