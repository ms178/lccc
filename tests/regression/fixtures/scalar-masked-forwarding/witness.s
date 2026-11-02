.text
.globl f0
.type f0,@function
f0:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    subq $64, %rsp
    vmovdqu64 (%rsi), %zmm6
    vmovdqu64 (%rdx), %zmm7
    kmovw %ecx, %k1
    vmovsd %xmm6, -32(%rbp)
    vmovsd %xmm7, -48(%rbp)
    vmovsd -32(%rbp), %xmm7{%k1}
    vmovdqu64 %zmm7, (%rdi)
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
    vmovdqu64 (%rsi), %zmm6
    vmovdqu64 (%rdx), %zmm7
    kmovw %ecx, %k1
    vmovsd %xmm6, -32(%rbp)
    vmovsd %xmm7, -48(%rbp)
    vmovsd -32(%rbp), %xmm7{%k1}
    vmovsd -48(%rbp), %xmm9
    vmovdqu64 %zmm9, (%rdi)
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
    vmovdqu64 (%rsi), %zmm6
    vmovdqu64 (%rdx), %zmm7
    kmovw %ecx, %k1
    vmovsd %xmm6, -32(%rbp)
    vmovsd %xmm7, -48(%rbp)
    vmovsd -32(%rbp), %xmm7{%k1}{z}
    vmovdqu64 %zmm7, (%rdi)
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
    vmovdqu64 (%rsi), %zmm6
    vmovdqu64 (%rdx), %zmm7
    kmovw %ecx, %k1
    vmovsd %xmm6, -32(%rbp)
    vmovsd %xmm7, -48(%rbp)
    vmovsd -32(%rbp), %xmm7{%k1}{z}
    vmovsd -48(%rbp), %xmm9
    vmovdqu64 %zmm9, (%rdi)
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
    vmovdqu64 (%rsi), %zmm6
    vmovdqu64 (%rdx), %zmm7
    kmovw %ecx, %k1
    vmovss %xmm6, -32(%rbp)
    vmovss %xmm7, -48(%rbp)
    vmovss -32(%rbp), %xmm7{%k1}
    vmovdqu64 %zmm7, (%rdi)
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
    vmovdqu64 (%rsi), %zmm6
    vmovdqu64 (%rdx), %zmm7
    kmovw %ecx, %k1
    vmovss %xmm6, -32(%rbp)
    vmovss %xmm7, -48(%rbp)
    vmovss -32(%rbp), %xmm7{%k1}
    vmovss -48(%rbp), %xmm9
    vmovdqu64 %zmm9, (%rdi)
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
    vmovdqu64 (%rsi), %zmm6
    vmovdqu64 (%rdx), %zmm7
    kmovw %ecx, %k1
    vmovss %xmm6, -32(%rbp)
    vmovss %xmm7, -48(%rbp)
    vmovss -32(%rbp), %xmm7{%k1}{z}
    vmovdqu64 %zmm7, (%rdi)
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
    vmovdqu64 (%rsi), %zmm6
    vmovdqu64 (%rdx), %zmm7
    kmovw %ecx, %k1
    vmovss %xmm6, -32(%rbp)
    vmovss %xmm7, -48(%rbp)
    vmovss -32(%rbp), %xmm7{%k1}{z}
    vmovss -48(%rbp), %xmm9
    vmovdqu64 %zmm9, (%rdi)
    leave
    ret
.cfi_endproc
.size f7,.-f7
.section .note.GNU-stack,"",@progbits
