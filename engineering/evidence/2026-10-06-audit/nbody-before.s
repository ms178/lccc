.section .rodata
.Lstr0:
    .byte 37, 46, 57, 102, 10, 0

.section .data
.align 16
.type bodies, @object
.size bodies, 280
bodies:
    .zero 48
    .byte 222
    .byte 69
    .byte 190
    .byte 201
    .byte 60
    .byte 189
    .byte 67
    .byte 64
    .byte 44
    .byte 217
    .byte 60
    .byte 52
    .byte 160
    .byte 93
    .byte 19
    .byte 64
    .byte 124
    .byte 219
    .byte 31
    .byte 192
    .byte 171
    .byte 144
    .byte 242
    .byte 191
    .byte 240
    .byte 235
    .byte 37
    .byte 108
    .byte 249
    .byte 134
    .byte 186
    .byte 191
    .byte 188
    .byte 204
    .byte 147
    .byte 155
    .byte 6
    .byte 103
    .byte 227
    .byte 63
    .byte 155
    .byte 148
    .byte 125
    .byte 245
    .byte 242
    .byte 126
    .byte 6
    .byte 64
    .byte 21
    .byte 7
    .byte 90
    .byte 154
    .byte 215
    .byte 210
    .byte 153
    .byte 191
    .byte 216
    .byte 51
    .byte 171
    .byte 217
    .byte 149
    .byte 76
    .byte 163
    .byte 63
    .byte 103
    .byte 202
    .byte 50
    .byte 195
    .byte 205
    .byte 175
    .byte 32
    .byte 64
    .byte 176
    .byte 1
    .byte 222
    .byte 49
    .byte 203
    .byte 127
    .byte 16
    .byte 64
    .byte 124
    .byte 70
    .byte 235
    .byte 225
    .byte 83
    .byte 211
    .byte 217
    .byte 191
    .byte 66
    .byte 148
    .byte 135
    .byte 184
    .byte 33
    .byte 44
    .byte 240
    .byte 191
    .byte 19
    .byte 143
    .byte 31
    .byte 191
    .byte 233
    .byte 53
    .byte 253
    .byte 63
    .byte 180
    .byte 35
    .byte 17
    .byte 95
    .byte 72
    .byte 60
    .byte 129
    .byte 63
    .byte 55
    .byte 198
    .byte 7
    .byte 13
    .byte 73
    .byte 29
    .byte 135
    .byte 63
    .byte 207
    .byte 217
    .byte 167
    .byte 206
    .byte 234
    .byte 201
    .byte 41
    .byte 64
    .byte 126
    .byte 102
    .byte 38
    .byte 214
    .byte 232
    .byte 56
    .byte 46
    .byte 192
    .byte 160
    .byte 125
    .byte 37
    .byte 190
    .byte 87
    .byte 149
    .byte 204
    .byte 191
    .byte 239
    .byte 27
    .byte 145
    .byte 169
    .byte 28
    .byte 83
    .byte 241
    .byte 63
    .byte 197
    .byte 187
    .byte 84
    .byte 62
    .byte 127
    .byte 204
    .byte 235
    .byte 63
    .byte 124
    .byte 62
    .byte 242
    .byte 250
    .byte 107
    .byte 47
    .byte 134
    .byte 191
    .byte 179
    .byte 30
    .byte 244
    .byte 156
    .byte 210
    .byte 61
    .byte 92
    .byte 63
    .byte 42
    .byte 87
    .byte 5
    .byte 169
    .byte 103
    .byte 194
    .byte 46
    .byte 64
    .byte 32
    .byte 162
    .byte 200
    .byte 51
    .byte 88
    .byte 235
    .byte 57
    .byte 192
    .byte 64
    .byte 229
    .byte 171
    .byte 147
    .byte 243
    .byte 241
    .byte 198
    .byte 63
    .byte 74
    .byte 188
    .byte 89
    .byte 22
    .byte 182
    .byte 84
    .byte 239
    .byte 63
    .byte 163
    .byte 251
    .byte 196
    .byte 49
    .byte 198
    .byte 7
    .byte 227
    .byte 63
    .byte 246
    .byte 101
    .byte 118
    .byte 88
    .byte 136
    .byte 203
    .byte 161
    .byte 191
    .byte 172
    .byte 153
    .byte 23
    .byte 83
    .byte 243
    .byte 168
    .byte 96
    .byte 63

.text
.globl main
.p2align 4
.type main, @function
main:
.cfi_startproc
    pushq %rbp
    .cfi_def_cfa_offset 16
    .cfi_offset %rbp, -16
    movq %rsp, %rbp
    .cfi_def_cfa_register %rbp
    pushq %rbx
    .cfi_offset %rbx, -24
    pushq %r12
    .cfi_offset %r12, -32
    pushq %r13
    .cfi_offset %r13, -40
    pushq %r14
    .cfi_offset %r14, -48
    pushq %r15
    .cfi_offset %r15, -56
    subq $136, %rsp
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    xorpd %xmm0, %xmm0
    movdqu %xmm0, -64(%rbp)
    movsd .LCFP_0(%rip), %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm15
    leaq bodies(%rip), %rbx
    movsd bodies+48(%rip), %xmm2
    vmovddup %xmm2, %xmm14
    movupd 24(%rbx), %xmm13
    movdqu -64(%rbp), %xmm3
    vfmadd132pd %xmm14, %xmm3, %xmm13
    movsd bodies+40(%rip), %xmm3
    xorpd %xmm0, %xmm0
    vfmadd132sd %xmm2, %xmm0, %xmm3
    movsd bodies+104(%rip), %xmm4
    vmovddup %xmm4, %xmm14
    movupd 80(%rbx), %xmm12
    vfmadd132pd %xmm14, %xmm13, %xmm12
    vfmadd231sd bodies+96(%rip), %xmm4, %xmm3
    movsd bodies+160(%rip), %xmm6
    vmovddup %xmm6, %xmm14
    movupd 136(%rbx), %xmm13
    vfmadd132pd %xmm14, %xmm12, %xmm13
    vfmadd231sd bodies+152(%rip), %xmm6, %xmm3
    movsd bodies+216(%rip), %xmm8
    vmovddup %xmm8, %xmm14
    movupd 192(%rbx), %xmm12
    vfmadd132pd %xmm14, %xmm13, %xmm12
    vfmadd231sd bodies+208(%rip), %xmm8, %xmm3
    movsd bodies+272(%rip), %xmm10
    vmovddup %xmm10, %xmm14
    movupd 248(%rbx), %xmm13
    vfmadd132pd %xmm14, %xmm12, %xmm13
    vfmadd231sd bodies+264(%rip), %xmm10, %xmm3
    vxorpd .LCVEC_1(%rip), %xmm13, %xmm13
    vdivpd %xmm15, %xmm13, %xmm13
    movupd %xmm13, 24(%rbx)
    vxorpd .LCFP_2(%rip), %xmm3, %xmm2
    vdivsd .LCFP_0(%rip), %xmm2, %xmm2
    movsd %xmm2, bodies+40(%rip)
    movsd .LCFP_3(%rip), %xmm3
    xorpd %xmm4, %xmm4
    xorl %r11d, %r11d
    movq %rbx, %r10
.LBB1:
    cmpl $5, %r11d
jge .LBB6
.p2align 4,,10
.p2align 3
.LBB2:
    leaq 48(%r10), %r12
    movsd (%r12), %xmm5
    vmulsd %xmm3, %xmm5, %xmm5
    movsd 24(%r10), %xmm6
    vmulsd %xmm6, %xmm6, %xmm6
    movsd 32(%r10), %xmm7
    vfmadd231sd %xmm7, %xmm7, %xmm6
    movsd 40(%r10), %xmm8
    vfmadd231sd %xmm8, %xmm8, %xmm6
    vmulsd %xmm6, %xmm5, %xmm5
    leal 1(%r11), %eax
    movslq %eax, %r9
    leaq 16(%r10), %r13
    leaq 8(%r10), %rsi
    imulq $56, %r9, %r8
    leaq bodies(%rip), %rcx
    leaq (%rcx, %r8), %r14
    vaddsd %xmm5, %xmm4, %xmm9
    movq %r14, %r8
.LBB3:
    cmpq $5, %r9
jge .LBB5
.p2align 4,,10
.p2align 3
.LBB4:
    movsd (%r10), %xmm10
    vsubsd (%r8), %xmm10, %xmm10
    movsd (%rsi), %xmm2
    vsubsd 8(%r8), %xmm2, %xmm2
    movsd (%r13), %xmm5
    vsubsd 16(%r8), %xmm5, %xmm5
    movsd (%r12), %xmm7
    vmulsd 48(%r8), %xmm7, %xmm7
    vmulsd %xmm10, %xmm10, %xmm10
    vfmadd231sd %xmm2, %xmm2, %xmm10
    vfmadd231sd %xmm5, %xmm5, %xmm10
    vsqrtsd %xmm10, %xmm10, %xmm10
    vdivsd %xmm10, %xmm7, %xmm7
    vsubsd %xmm7, %xmm9, %xmm9
    addq $1, %r9
    leaq 56(%r8), %r8
    cmpq $5, %r9
jl .LBB4
.LBB5:
    addl $1, %r11d
    leaq 56(%r10), %rax
    movq %rax, -160(%rbp)
    vmovapd %xmm9, %xmm4
    movq %rax, %r10
    cmpl $5, %r11d
jl .LBB2
.LBB6:
    leaq .Lstr0(%rip), %rdi
    vmovsd %xmm4, %xmm4, %xmm0
    movb $1, %al
    call printf@PLT
    # LCCC_VA_CALL
    # LCCC_CALL_ARGS 1
    # LCCC_CALL_FP 1
    leaq 248(%rbx), %rax
    movq %rax, -80(%rbp)
    leaq 224(%rbx), %rax
    movq %rax, -88(%rbp)
    leaq 192(%rbx), %rax
    movq %rax, -96(%rbp)
    leaq 168(%rbx), %rax
    movq %rax, -104(%rbp)
    leaq 136(%rbx), %rax
    movq %rax, -112(%rbp)
    leaq 112(%rbx), %rax
    movq %rax, -120(%rbp)
    leaq 80(%rbx), %rax
    movq %rax, -128(%rbp)
    leaq 56(%rbx), %rax
    movq %rax, -136(%rbp)
    leaq 24(%rbx), %rax
    movq %rax, -144(%rbp)
    movsd .LCFP_4(%rip), %xmm0
    unpcklpd %xmm0, %xmm0
    movdqa %xmm0, %xmm13
    movsd .LCFP_4(%rip), %xmm11
    movq $0, -72(%rbp)
.p2align 5,,15
.p2align 4
.LBB7:
    cmpl $5000000, -72(%rbp)
    movsd .LCFP_3(%rip), %xmm2
    jge .LBB15
.LBB8:
    xorl %r13d, %r13d
.LBB9:
    cmpl $5, %r13d
jge .LBB14
.p2align 5,,15
.p2align 4
.LBB10:
    movslq %r13d, %r9
    imulq $56, %r9, %r15
    leaq bodies(%rip), %rcx
    leaq (%rcx, %r15), %r12
    leal 1(%r13), %r9d
.LBB11:
    cmpl $5, %r9d
jge .LBB13
.p2align 5,,15
.p2align 4
.LBB12:
    movslq %r9d, %r11
    imulq $56, %r11, %r11
    leaq bodies(%rip), %rcx
    movupd (%rbx,%r15), %xmm15
    vsubpd (%rbx,%r11), %xmm15, %xmm15
    movapd %xmm15, %xmm0
    movsd %xmm15, -160(%rbp)
    pshufd $0x0E, %xmm15, %xmm0
    movsd %xmm0, -168(%rbp)
    movsd 16(%r12), %xmm3
    vsubsd 16(%rcx, %r11), %xmm3, %xmm3
    movsd -160(%rbp), %xmm5
    vmulsd %xmm5, %xmm5, %xmm5
    movsd %xmm5, %xmm0
    movsd -168(%rbp), %xmm1
    vfmadd231sd %xmm1, %xmm1, %xmm0
    movsd %xmm0, %xmm5
    vfmadd231sd %xmm3, %xmm3, %xmm5
    vsqrtsd %xmm5, %xmm5, %xmm8
    vmulsd %xmm8, %xmm5, %xmm5
    vdivsd %xmm5, %xmm11, %xmm9
    vmovddup %xmm9, %xmm14
    movsd 48(%rcx, %r11), %xmm10
    vmovddup %xmm10, %xmm12
    vmulpd %xmm12, %xmm15, %xmm12
    vfnmadd213pd 24(%rbx,%r15), %xmm14, %xmm12
    movupd %xmm12, 24(%rbx,%r15)
    vmulsd %xmm10, %xmm3, %xmm4
    movsd 40(%r12), %xmm5
    vfnmadd231sd %xmm9, %xmm4, %xmm5
    movsd %xmm5, 40(%r12)
    movsd 48(%r12), %xmm6
    vmovddup %xmm6, %xmm12
    vmulpd %xmm12, %xmm15, %xmm15
    vfmadd213pd 24(%rbx,%r11), %xmm14, %xmm15
    movupd %xmm15, 24(%rbx,%r11)
    vmulsd %xmm6, %xmm3, %xmm3
    movsd 40(%rcx, %r11), %xmm8
    vfmadd231sd %xmm9, %xmm3, %xmm8
    movsd %xmm8, 40(%rcx, %r11)
    addl $1, %r9d
    cmpl $5, %r9d
jl .LBB12
.LBB13:
    addl $1, %r13d
    cmpl $5, %r13d
jl .LBB10
.LBB14:
    movq -144(%rbp), %rax
    movupd (%rax), %xmm15
    vfmadd213pd (%rbx), %xmm13, %xmm15
    movupd %xmm15, (%rbx)
    movsd bodies+40(%rip), %xmm10
    movsd bodies+16(%rip), %xmm2
    vfmadd231sd %xmm11, %xmm10, %xmm2
    movsd %xmm2, bodies+16(%rip)
    movq -128(%rbp), %rax
    movupd (%rax), %xmm15
    movq -136(%rbp), %rax
    movupd (%rax), %xmm3
    vfmadd132pd %xmm13, %xmm3, %xmm15
    movupd %xmm15, (%rax)
    movsd bodies+96(%rip), %xmm4
    movsd bodies+72(%rip), %xmm5
    vfmadd231sd %xmm11, %xmm4, %xmm5
    movsd %xmm5, bodies+72(%rip)
    movq -112(%rbp), %rax
    movupd (%rax), %xmm15
    movq -120(%rbp), %rax
    movupd (%rax), %xmm6
    vfmadd132pd %xmm13, %xmm6, %xmm15
    movupd %xmm15, (%rax)
    movsd bodies+152(%rip), %xmm7
    movsd bodies+128(%rip), %xmm8
    vfmadd231sd %xmm11, %xmm7, %xmm8
    movsd %xmm8, bodies+128(%rip)
    movq -96(%rbp), %rax
    movupd (%rax), %xmm15
    movq -104(%rbp), %rax
    movupd (%rax), %xmm9
    vfmadd132pd %xmm13, %xmm9, %xmm15
    movupd %xmm15, (%rax)
    movsd bodies+208(%rip), %xmm10
    movsd bodies+184(%rip), %xmm2
    vfmadd231sd %xmm11, %xmm10, %xmm2
    movsd %xmm2, bodies+184(%rip)
    movq -80(%rbp), %rax
    movupd (%rax), %xmm15
    movq -88(%rbp), %rax
    movupd (%rax), %xmm3
    vfmadd132pd %xmm13, %xmm3, %xmm15
    movupd %xmm15, (%rax)
    movsd bodies+264(%rip), %xmm4
    movsd bodies+240(%rip), %xmm5
    vfmadd231sd %xmm11, %xmm4, %xmm5
    movsd %xmm5, bodies+240(%rip)
    movl -72(%rbp), %eax
    addl $1, %eax
    movq %rax, -72(%rbp)
    jmp .LBB7
.LBB15:
    xorpd %xmm6, %xmm6
    xorl %r10d, %r10d
    leaq bodies(%rip), %r8
.LBB16:
    cmpl $5, %r10d
jge .LBB21
.p2align 4,,10
.p2align 3
.LBB17:
    leaq 48(%r8), %r15
    movsd (%r15), %xmm7
    vmulsd %xmm2, %xmm7, %xmm7
    movsd 24(%r8), %xmm8
    vmulsd %xmm8, %xmm8, %xmm8
    movsd 32(%r8), %xmm9
    vfmadd231sd %xmm9, %xmm9, %xmm8
    movsd 40(%r8), %xmm10
    vfmadd231sd %xmm10, %xmm10, %xmm8
    vmulsd %xmm8, %xmm7, %xmm7
    leal 1(%r10), %eax
    movslq %eax, %rdi
    leaq 16(%r8), %r12
    leaq 8(%r8), %r11
    imulq $56, %rdi, %r9
    leaq bodies(%rip), %r13
    addq %r9, %r13
    vaddsd %xmm7, %xmm6, %xmm11
    movq %r13, %r9
.LBB18:
    cmpq $5, %rdi
jge .LBB20
.p2align 4,,10
.p2align 3
.LBB19:
    movsd (%r8), %xmm3
    vsubsd (%r9), %xmm3, %xmm3
    movsd (%r11), %xmm5
    vsubsd 8(%r9), %xmm5, %xmm5
    movsd (%r12), %xmm7
    vsubsd 16(%r9), %xmm7, %xmm7
    movsd (%r15), %xmm9
    vmulsd 48(%r9), %xmm9, %xmm9
    vmulsd %xmm3, %xmm3, %xmm3
    vfmadd231sd %xmm5, %xmm5, %xmm3
    vfmadd231sd %xmm7, %xmm7, %xmm3
    vsqrtsd %xmm3, %xmm3, %xmm3
    vdivsd %xmm3, %xmm9, %xmm9
    vsubsd %xmm9, %xmm11, %xmm11
    addq $1, %rdi
    leaq 56(%r9), %r9
    cmpq $5, %rdi
jl .LBB19
.LBB20:
    addl $1, %r10d
    leaq 56(%r8), %rax
    movq %rax, -160(%rbp)
    vmovapd %xmm11, %xmm6
    movq %rax, %r8
    cmpl $5, %r10d
jl .LBB17
.LBB21:
    leaq .Lstr0(%rip), %rdi
    vmovsd %xmm6, %xmm6, %xmm0
    movb $1, %al
    call printf@PLT
    # LCCC_VA_CALL
    # LCCC_CALL_ARGS 1
    # LCCC_CALL_FP 1
    xorl %eax, %eax
    leaq -40(%rbp), %rsp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    popq %rbp
    .cfi_def_cfa %rsp, 8
    ret
.cfi_endproc
.size main, .-main

.section .rodata
.p2align 4
.LCFP_0:
    .quad 4630752910647379422
    .quad 0
.p2align 4
.LCFP_2:
    .quad -9223372036854775808
    .quad 0
.p2align 4
.LCFP_3:
    .quad 4602678819172646912
    .quad 0
.p2align 4
.LCFP_4:
    .quad 4576918229304087675
    .quad 0
.p2align 4
.LCVEC_1:
    .quad -9223372036854775808
    .quad -9223372036854775808

.section .note.GNU-stack,"",@progbits
