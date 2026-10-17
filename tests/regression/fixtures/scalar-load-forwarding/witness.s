.text
.globl f0
.type f0,@function
f0:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movsd %xmm6, -32(%rbp)
    movsd -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f0,.-f0
.globl f1
.type f1,@function
f1:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movq %rax, -32(%rbp)
    movsd -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f1,.-f1
.globl f2
.type f2,@function
f2:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movsd %xmm6, -32(%rbp)
    movsd -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f2,.-f2
.globl f3
.type f3,@function
f3:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movq %rax, -32(%rbp)
    movsd -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f3,.-f3
.globl f4
.type f4,@function
f4:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movq %xmm6, -32(%rbp)
    movq -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f4,.-f4
.globl f5
.type f5,@function
f5:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movq %rax, -32(%rbp)
    movq -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f5,.-f5
.globl f6
.type f6,@function
f6:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movq %xmm6, -32(%rbp)
    movq -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f6,.-f6
.globl f7
.type f7,@function
f7:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movq %rax, -32(%rbp)
    movq -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f7,.-f7
.globl f8
.type f8,@function
f8:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movss %xmm6, -32(%rbp)
    movss -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f8,.-f8
.globl f9
.type f9,@function
f9:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movl %eax, -32(%rbp)
    movss -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f9,.-f9
.globl f10
.type f10,@function
f10:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movss %xmm6, -32(%rbp)
    movss -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f10,.-f10
.globl f11
.type f11,@function
f11:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movl %eax, -32(%rbp)
    movss -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f11,.-f11
.globl f12
.type f12,@function
f12:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movd %xmm6, -32(%rbp)
    movd -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f12,.-f12
.globl f13
.type f13,@function
f13:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movl %eax, -32(%rbp)
    movd -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f13,.-f13
.globl f14
.type f14,@function
f14:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movd %xmm6, -32(%rbp)
    movd -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f14,.-f14
.globl f15
.type f15,@function
f15:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movl %eax, -32(%rbp)
    movd -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f15,.-f15
.globl f16
.type f16,@function
f16:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    vmovsd %xmm6, -32(%rbp)
    vmovsd -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f16,.-f16
.globl f17
.type f17,@function
f17:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movq %rax, -32(%rbp)
    vmovsd -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f17,.-f17
.globl f18
.type f18,@function
f18:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    vmovsd %xmm6, -32(%rbp)
    vmovsd -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f18,.-f18
.globl f19
.type f19,@function
f19:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movq %rax, -32(%rbp)
    vmovsd -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f19,.-f19
.globl f20
.type f20,@function
f20:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    vmovq %xmm6, -32(%rbp)
    vmovq -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f20,.-f20
.globl f21
.type f21,@function
f21:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movq %rax, -32(%rbp)
    vmovq -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f21,.-f21
.globl f22
.type f22,@function
f22:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    vmovq %xmm6, -32(%rbp)
    vmovq -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f22,.-f22
.globl f23
.type f23,@function
f23:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movq %rax, -32(%rbp)
    vmovq -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f23,.-f23
.globl f24
.type f24,@function
f24:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    vmovss %xmm6, -32(%rbp)
    vmovss -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f24,.-f24
.globl f25
.type f25,@function
f25:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movl %eax, -32(%rbp)
    vmovss -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f25,.-f25
.globl f26
.type f26,@function
f26:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    vmovss %xmm6, -32(%rbp)
    vmovss -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f26,.-f26
.globl f27
.type f27,@function
f27:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movl %eax, -32(%rbp)
    vmovss -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f27,.-f27
.globl f28
.type f28,@function
f28:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    vmovd %xmm6, -32(%rbp)
    vmovd -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f28,.-f28
.globl f29
.type f29,@function
f29:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movl %eax, -32(%rbp)
    vmovd -32(%rbp), %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f29,.-f29
.globl f30
.type f30,@function
f30:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    vmovd %xmm6, -32(%rbp)
    vmovd -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f30,.-f30
.globl f31
.type f31,@function
f31:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq (%rsi), %rax
    movl %eax, -32(%rbp)
    vmovd -32(%rbp), %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    leave
    ret
.cfi_endproc
.size f31,.-f31
.globl f32
.type f32,@function
f32:
.cfi_startproc
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq %xmm6, %rax
    movq %rax, %xmm7
    vmovdqu %ymm7, (%rdi)
    xorl %eax, %eax
    ret
.cfi_endproc
.size f32,.-f32
.globl f33
.type f33,@function
f33:
.cfi_startproc
    vmovdqu (%rsi), %ymm6
    vmovdqu (%rdx), %ymm7
    movq %xmm6, %rax
    movq %rax, %xmm6
    vmovdqu %ymm6, (%rdi)
    xorl %eax, %eax
    ret
.cfi_endproc
.size f33,.-f33
.globl f34
.type f34,@function
f34:
.cfi_startproc
    vmovdqu (%rsi), %ymm6
    movq %xmm6, %xmm6
    vmovdqu %ymm6, (%rdi)
    ret
.cfi_endproc
.size f34,.-f34
.section .note.GNU-stack,"",@progbits
