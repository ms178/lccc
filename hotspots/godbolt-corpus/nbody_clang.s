.LCPI0_0:
  .quad 0xc043bd3cc9be45de
  .quad 0xc043bd3cc9be45de
.LCPI0_1:
  .quad 0xc043bd3cc9be45de
.LCPI0_2:
  .quad 0x3fe0000000000000
.LCPI0_3:
  .quad 0x3f847ae147ae147b
main:
  pushq %rbx
  subq $48, %rsp
  vmovddup bodies+48(%rip), %xmm1
  vxorpd %xmm0, %xmm0, %xmm0
  vmovsd bodies+40(%rip), %xmm2
  vfmadd132sd %xmm1, %xmm0, %xmm2
  vmovddup bodies+104(%rip), %xmm3
  vfmadd231sd bodies+96(%rip), %xmm3, %xmm2
  vmovddup bodies+160(%rip), %xmm4
  vfmadd231sd bodies+152(%rip), %xmm4, %xmm2
  vmovddup bodies+216(%rip), %xmm5
  vfmadd231sd bodies+208(%rip), %xmm5, %xmm2
  vmovddup bodies+272(%rip), %xmm6
  vfmadd231sd bodies+264(%rip), %xmm6, %xmm2
  vxorpd %xmm7, %xmm7, %xmm7
  vfmadd231pd bodies+24(%rip), %xmm1, %xmm7
  vfmadd231pd bodies+80(%rip), %xmm3, %xmm7
  vfmadd231pd bodies+136(%rip), %xmm4, %xmm7
  vfmadd231pd bodies+192(%rip), %xmm5, %xmm7
  vfmadd231pd bodies+248(%rip), %xmm6, %xmm7
  vdivpd .LCPI0_0(%rip), %xmm7, %xmm1
  vmovupd %xmm1, bodies+24(%rip)
  vdivsd .LCPI0_1(%rip), %xmm2, %xmm1
  vmovsd %xmm1, bodies+40(%rip)
  movl $1, %eax
  xorl %ecx, %ecx
  leaq bodies(%rip), %rbx
  jmp .LBB0_2
.LBB0_1:
  incq %rcx
  incq %rax
  cmpq $5, %rcx
  je .LBB0_9
.LBB0_2:
  vmovapd %xmm0, %xmm2
  imulq $56, %rcx, %rdx
  vmovsd 48(%rdx,%rbx), %xmm1
  vmulsd .LCPI0_2(%rip), %xmm1, %xmm3
  vmovsd 24(%rdx,%rbx), %xmm0
  vmovsd 32(%rdx,%rbx), %xmm4
  vmulsd %xmm4, %xmm4, %xmm4
  vfmadd231sd %xmm0, %xmm0, %xmm4
  vmovsd 40(%rdx,%rbx), %xmm0
  vfmadd213sd %xmm4, %xmm0, %xmm0
  vfmadd213sd %xmm2, %xmm3, %xmm0
  cmpq $3, %rcx
  ja .LBB0_1
  addq %rbx, %rdx
  vmovsd (%rdx), %xmm2
  vmovsd 8(%rdx), %xmm3
  vmovsd 16(%rdx), %xmm4
  testb $1, %cl
  jne .LBB0_5
  movq %rax, %rdx
  cmpq $3, %rcx
  je .LBB0_1
  jmp .LBB0_7
.LBB0_5:
  imulq $56, %rax, %rdx
  vsubsd (%rdx,%rbx), %xmm2, %xmm5
  vsubsd 8(%rdx,%rbx), %xmm3, %xmm6
  vsubsd 16(%rdx,%rbx), %xmm4, %xmm7
  vmulsd 48(%rdx,%rbx), %xmm1, %xmm8
  vmulsd %xmm6, %xmm6, %xmm6
  vfmadd231sd %xmm5, %xmm5, %xmm6
  vfmadd231sd %xmm7, %xmm7, %xmm6
  vsqrtsd %xmm6, %xmm6, %xmm5
  vdivsd %xmm5, %xmm8, %xmm5
  vsubsd %xmm5, %xmm0, %xmm0
  leaq 1(%rax), %rdx
  cmpq $3, %rcx
  je .LBB0_1
.LBB0_7:
  imulq $56, %rdx, %rdx
.LBB0_8:
  vsubsd (%rdx,%rbx), %xmm2, %xmm5
  vsubsd 8(%rdx,%rbx), %xmm3, %xmm6
  vsubsd 16(%rdx,%rbx), %xmm4, %xmm7
  vmulsd 48(%rdx,%rbx), %xmm1, %xmm8
  vsubsd 56(%rdx,%rbx), %xmm2, %xmm9
  vmulsd %xmm6, %xmm6, %xmm6
  vsubsd 64(%rdx,%rbx), %xmm3, %xmm10
  vfmadd231sd %xmm5, %xmm5, %xmm6
  vfmadd231sd %xmm7, %xmm7, %xmm6
  vsubsd 72(%rdx,%rbx), %xmm4, %xmm5
  vsqrtsd %xmm6, %xmm6, %xmm6
  vmulsd 104(%rdx,%rbx), %xmm1, %xmm7
  vmulsd %xmm10, %xmm10, %xmm10
  vdivsd %xmm6, %xmm8, %xmm6
  vfmadd231sd %xmm9, %xmm9, %xmm10
  vfmadd231sd %xmm5, %xmm5, %xmm10
  vsqrtsd %xmm10, %xmm10, %xmm5
  vsubsd %xmm6, %xmm0, %xmm0
  vdivsd %xmm5, %xmm7, %xmm5
  vsubsd %xmm5, %xmm0, %xmm0
  addq $112, %rdx
  cmpq $280, %rdx
  jne .LBB0_8
  jmp .LBB0_1
.LBB0_9:
  leaq .L.str(%rip), %rdi
  movb $1, %al
  callq printf@PLT
  vmovapd bodies(%rip), %xmm0
  vmovsd bodies+16(%rip), %xmm1
  vmovupd bodies+56(%rip), %xmm2
  vmovsd bodies+72(%rip), %xmm3
  vmovapd bodies+112(%rip), %xmm4
  vmovsd bodies+128(%rip), %xmm5
  vmovupd bodies+168(%rip), %xmm6
  vmovsd bodies+184(%rip), %xmm7
  vmovapd bodies+224(%rip), %xmm8
  xorl %eax, %eax
  vmovsd bodies+240(%rip), %xmm9
  leaq bodies+104(%rip), %rcx
  vmovsd .LCPI0_3(%rip), %xmm10
  jmp .LBB0_10
.LBB0_13:
  vmovapd 32(%rsp), %xmm0
  vmovddup .LCPI0_3(%rip), %xmm11
  vfmadd231pd bodies+24(%rip), %xmm11, %xmm0
  vmovapd %xmm0, bodies(%rip)
  vmovsd 8(%rsp), %xmm1
  vfmadd231sd bodies+40(%rip), %xmm10, %xmm1
  vmovsd %xmm1, bodies+16(%rip)
  vmovapd 16(%rsp), %xmm2
  vfmadd231pd bodies+80(%rip), %xmm11, %xmm2
  vmovupd %xmm2, bodies+56(%rip)
  vmovsd (%rsp), %xmm3
  vfmadd231sd bodies+96(%rip), %xmm10, %xmm3
  vmovsd %xmm3, bodies+72(%rip)
  vfmadd231pd bodies+136(%rip), %xmm11, %xmm4
  vmovapd %xmm4, bodies+112(%rip)
  vfmadd231sd bodies+152(%rip), %xmm10, %xmm5
  vmovsd %xmm5, bodies+128(%rip)
  vfmadd231pd bodies+192(%rip), %xmm11, %xmm6
  vmovupd %xmm6, bodies+168(%rip)
  vfmadd231sd bodies+208(%rip), %xmm10, %xmm7
  vmovsd %xmm7, bodies+184(%rip)
  vfmadd231pd bodies+248(%rip), %xmm11, %xmm8
  vmovapd %xmm8, bodies+224(%rip)
  vfmadd231sd bodies+264(%rip), %xmm10, %xmm9
  vmovsd %xmm9, bodies+240(%rip)
  incl %eax
  cmpl $5000000, %eax
  je .LBB0_14
.LBB0_10:
  vmovsd %xmm3, (%rsp)
  vmovapd %xmm2, 16(%rsp)
  vmovsd %xmm1, 8(%rsp)
  vmovapd %xmm0, 32(%rsp)
  movl $4, %edx
  movq %rcx, %rsi
  xorl %edi, %edi
  jmp .LBB0_11
.LBB0_12:
  incq %rdi
  decq %rdx
  addq $56, %rsi
  cmpq $5, %rdi
  je .LBB0_13
.LBB0_11:
  cmpq $3, %rdi
  ja .LBB0_12
  imulq $56, %rdi, %r9
  leaq (%rbx,%r9), %r8
  vmovupd (%r9,%rbx), %xmm12
  vmovsd 16(%r9,%rbx), %xmm13
  vmovddup 48(%r9,%rbx), %xmm14
  movq %rsi, %r9
  movq %rdx, %r10
.LBB0_16:
  vsubsd -32(%r9), %xmm13, %xmm15
  vmovddup (%r9), %xmm11
  vsubpd -48(%r9), %xmm12, %xmm0
  vmulsd %xmm11, %xmm15, %xmm1
  vmulpd %xmm0, %xmm0, %xmm2
  vshufpd $1, %xmm2, %xmm2, %xmm2
  vfmadd231sd %xmm0, %xmm0, %xmm2
  vfmadd231sd %xmm15, %xmm15, %xmm2
  vsqrtsd %xmm2, %xmm2, %xmm3
  vmulsd %xmm3, %xmm2, %xmm2
  vdivsd %xmm2, %xmm10, %xmm2
  vmulpd %xmm0, %xmm11, %xmm3
  vmovddup %xmm2, %xmm11
  vfnmadd213pd 24(%r8), %xmm11, %xmm3
  vmovupd %xmm3, 24(%r8)
  vfnmadd213sd 40(%r8), %xmm2, %xmm1
  vmovsd %xmm1, 40(%r8)
  vmulpd %xmm0, %xmm14, %xmm0
  vfmadd213pd -24(%r9), %xmm11, %xmm0
  vmovupd %xmm0, -24(%r9)
  vmulsd %xmm15, %xmm14, %xmm0
  vfmadd213sd -8(%r9), %xmm2, %xmm0
  vmovsd %xmm0, -8(%r9)
  addq $56, %r9
  decq %r10
  jne .LBB0_16
  jmp .LBB0_12
.LBB0_14:
  vxorpd %xmm0, %xmm0, %xmm0
  movl $1, %eax
  xorl %ecx, %ecx
  vmovsd .LCPI0_2(%rip), %xmm11
  jmp .LBB0_18
.LBB0_17:
  incq %rcx
  incq %rax
  cmpq $5, %rcx
  je .LBB0_25
.LBB0_18:
  vmovapd %xmm0, %xmm2
  imulq $56, %rcx, %rdx
  vmovsd 48(%rdx,%rbx), %xmm1
  vmulsd %xmm1, %xmm11, %xmm3
  vmovsd 24(%rdx,%rbx), %xmm0
  vmovsd 32(%rdx,%rbx), %xmm4
  vmulsd %xmm4, %xmm4, %xmm4
  vfmadd231sd %xmm0, %xmm0, %xmm4
  vmovsd 40(%rdx,%rbx), %xmm0
  vfmadd213sd %xmm4, %xmm0, %xmm0
  vfmadd213sd %xmm2, %xmm3, %xmm0
  cmpq $3, %rcx
  ja .LBB0_17
  addq %rbx, %rdx
  vmovsd (%rdx), %xmm2
  vmovsd 8(%rdx), %xmm3
  vmovsd 16(%rdx), %xmm4
  testb $1, %cl
  jne .LBB0_21
  movq %rax, %rdx
  cmpq $3, %rcx
  je .LBB0_17
  jmp .LBB0_23
.LBB0_21:
  imulq $56, %rax, %rdx
  vsubsd (%rdx,%rbx), %xmm2, %xmm5
  vsubsd 8(%rdx,%rbx), %xmm3, %xmm6
  vsubsd 16(%rdx,%rbx), %xmm4, %xmm7
  vmulsd 48(%rdx,%rbx), %xmm1, %xmm8
  vmulsd %xmm6, %xmm6, %xmm6
  vfmadd231sd %xmm5, %xmm5, %xmm6
  vfmadd231sd %xmm7, %xmm7, %xmm6
  vsqrtsd %xmm6, %xmm6, %xmm5
  vdivsd %xmm5, %xmm8, %xmm5
  vsubsd %xmm5, %xmm0, %xmm0
  leaq 1(%rax), %rdx
  cmpq $3, %rcx
  je .LBB0_17
.LBB0_23:
  imulq $56, %rdx, %rdx
.LBB0_24:
  vsubsd (%rdx,%rbx), %xmm2, %xmm5
  vsubsd 8(%rdx,%rbx), %xmm3, %xmm6
  vsubsd 16(%rdx,%rbx), %xmm4, %xmm7
  vmulsd 48(%rdx,%rbx), %xmm1, %xmm8
  vsubsd 56(%rdx,%rbx), %xmm2, %xmm9
  vmulsd %xmm6, %xmm6, %xmm6
  vsubsd 64(%rdx,%rbx), %xmm3, %xmm10
  vfmadd231sd %xmm5, %xmm5, %xmm6
  vfmadd231sd %xmm7, %xmm7, %xmm6
  vsubsd 72(%rdx,%rbx), %xmm4, %xmm5
  vsqrtsd %xmm6, %xmm6, %xmm6
  vmulsd 104(%rdx,%rbx), %xmm1, %xmm7
  vmulsd %xmm10, %xmm10, %xmm10
  vdivsd %xmm6, %xmm8, %xmm6
  vfmadd231sd %xmm9, %xmm9, %xmm10
  vfmadd231sd %xmm5, %xmm5, %xmm10
  vsqrtsd %xmm10, %xmm10, %xmm5
  vsubsd %xmm6, %xmm0, %xmm0
  vdivsd %xmm5, %xmm7, %xmm5
  vsubsd %xmm5, %xmm0, %xmm0
  addq $112, %rdx
  cmpq $280, %rdx
  jne .LBB0_24
  jmp .LBB0_17
.LBB0_25:
  leaq .L.str(%rip), %rdi
  movb $1, %al
  callq printf@PLT
  xorl %eax, %eax
  addq $48, %rsp
  popq %rbx
  retq

bodies:
  .quad 0x0000000000000000
  .quad 0x0000000000000000
  .quad 0x0000000000000000
  .quad 0x0000000000000000
  .quad 0x0000000000000000
  .quad 0x0000000000000000
  .quad 0x4043bd3cc9be45de
  .quad 0x40135da0343cd92c
  .quad 0xbff290abc01fdb7c
  .quad 0xbfba86f96c25ebf0
  .quad 0x3fe367069b93ccbc
  .quad 0x40067ef2f57d949b
  .quad 0xbf99d2d79a5a0715
  .quad 0x3fa34c95d9ab33d8
  .quad 0x4020afcdc332ca67
  .quad 0x40107fcb31de01b0
  .quad 0xbfd9d353e1eb467c
  .quad 0xbff02c21b8879442
  .quad 0x3ffd35e9bf1f8f13
  .quad 0x3f813c485f1123b4
  .quad 0x3f871d490d07c637
  .quad 0x4029c9eacea7d9cf
  .quad 0xc02e38e8d626667e
  .quad 0xbfcc9557be257da0
  .quad 0x3ff1531ca9911bef
  .quad 0x3febcc7f3e54bbc5
  .quad 0xbf862f6bfaf23e7c
  .quad 0x3f5c3dd29cf41eb3
  .quad 0x402ec267a905572a
  .quad 0xc039eb5833c8a220
  .quad 0x3fc6f1f393abe540
  .quad 0x3fef54b61659bc4a
  .quad 0x3fe307c631c4fba3
  .quad 0xbfa1cb88587665f6
  .quad 0x3f60a8f3531799ac

.L.str:
  .asciz "%.9f\n"

