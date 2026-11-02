.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
.LCPI0_3:
.LCPI0_8:
.LCPI0_4:
.LCPI0_5:
.LCPI0_6:
.LCPI0_7:
main:
  pushq %r15
  pushq %r14
  pushq %rbx
  subq $688, %rsp
  vstmxcsr 12(%rsp)
  orl $32832, 12(%rsp)
  vldmxcsr 12(%rsp)
  vmovddup bodies+48(%rip), %xmm9
  vmulsd bodies+40(%rip), %xmm9, %xmm0
  vmovsd bodies+96(%rip), %xmm5
  vmovsd bodies+152(%rip), %xmm2
  vmovsd %xmm2, 128(%rsp)
  vmovsd bodies+208(%rip), %xmm7
  vmovsd bodies+264(%rip), %xmm8
  vmulpd bodies+24(%rip), %xmm9, %xmm1
  vmovddup bodies+104(%rip), %xmm4
  vmovupd bodies+80(%rip), %xmm3
  vmovupd %xmm3, 112(%rsp)
  vfmadd231pd %xmm4, %xmm3, %xmm1
  vmovupd %xmm4, 240(%rsp)
  vmovupd %xmm5, 48(%rsp)
  vfmadd231sd %xmm4, %xmm5, %xmm0
  vmovddup bodies+272(%rip), %xmm3
  vmovddup bodies+216(%rip), %xmm4
  vmovddup bodies+160(%rip), %xmm5
  vmovupd bodies+136(%rip), %xmm6
  vmovupd %xmm6, 32(%rsp)
  vfmadd231pd %xmm5, %xmm6, %xmm1
  vmovupd %xmm5, 320(%rsp)
  vfmadd231sd %xmm5, %xmm2, %xmm0
  vmovupd bodies+192(%rip), %xmm10
  vfmadd231pd %xmm4, %xmm10, %xmm1
  vmovupd %xmm4, 336(%rsp)
  vmovupd %xmm7, 80(%rsp)
  vfmadd231sd %xmm4, %xmm7, %xmm0
  vmovupd bodies+248(%rip), %xmm12
  vfmadd231pd %xmm3, %xmm12, %xmm1
  vmovupd %xmm3, 352(%rsp)
  vmovupd %xmm8, 64(%rsp)
  vfmadd231sd %xmm3, %xmm8, %xmm0
  vmulpd .LCPI0_0(%rip), %xmm1, %xmm1
  vmovupd %xmm1, 176(%rsp)
  vmovupd %xmm1, bodies+24(%rip)
  vmulsd .LCPI0_1(%rip), %xmm0, %xmm11
  vmovsd %xmm11, bodies+40(%rip)
  vxorpd %xmm0, %xmm0, %xmm0
  movq $-224, %rax
  movl $1, %ecx
  xorl %esi, %esi
  movl $bodies, %edx
  vmovdqu .LCPI0_4(%rip), %ymm13
  vmovdqu .LCPI0_6(%rip), %ymm7
  vmovdqu .LCPI0_7(%rip), %ymm8
  vmovupd %xmm9, 256(%rsp)
  vmovupd %xmm10, 224(%rsp)
  vmovupd %xmm11, 208(%rsp)
  vmovupd %xmm12, 192(%rsp)
  vpbroadcastq .LCPI0_3(%rip), %ymm9
  jmp .LBB0_1
.LBB0_9:
  leaq 1(%rsi), %rdi
  addq $56, %rax
  incq %rcx
  cmpq $4, %rsi
  movq %rdi, %rsi
  je .LBB0_10
.LBB0_1:
  vmovapd %xmm0, %xmm1
  imulq $56, %rsi, %rdi
  vmovsd bodies+24(%rdi), %xmm2
  vmovsd bodies+32(%rdi), %xmm0
  vmovupd bodies+40(%rdi), %xmm6
  vmovhpd .LCPI0_2(%rip), %xmm6, %xmm4
  vmulpd %xmm4, %xmm6, %xmm4
  vfmadd213sd %xmm4, %xmm2, %xmm2
  vfmadd231sd %xmm0, %xmm0, %xmm2
  vshufpd $1, %xmm4, %xmm4, %xmm0
  vfmadd213sd %xmm1, %xmm2, %xmm0
  cmpq $3, %rsi
  ja .LBB0_9
  vmovsd bodies(%rdi), %xmm1
  vmovupd bodies+8(%rdi), %xmm2
  movl $4, %r8d
  subq %rsi, %r8
  movq %r8, %rdi
  andq $-4, %rdi
  je .LBB0_3
  leaq -1(%rdi), %r9
  vmovupd %xmm1, 16(%rsp)
  vbroadcastsd %xmm1, %ymm4
  vbroadcastsd %xmm2, %ymm5
  vmovdqa %ymm7, %ymm3
  vpermpd $85, %ymm2, %ymm7
  vmovdqa %ymm8, %ymm1
  vmovupd %ymm6, 288(%rsp)
  vpermpd $85, %ymm6, %ymm8
  vxorpd %xmm6, %xmm6, %xmm6
  vmovq %rdx, %xmm10
  vpbroadcastq %xmm10, %ymm10
  xorl %r10d, %r10d
.LBB0_7:
  leaq (%rsi,%r10), %r11
  vmovq %r11, %xmm11
  vpbroadcastq %xmm11, %ymm11
  vpmuludq %ymm9, %ymm11, %ymm12
  vpsrlq $32, %ymm11, %ymm11
  vpaddq %ymm10, %ymm12, %ymm12
  vpmuludq %ymm9, %ymm11, %ymm11
  vpsllq $32, %ymm11, %ymm11
  vpaddq %ymm11, %ymm12, %ymm11
  vpaddq %ymm13, %ymm11, %ymm12
  vmovq %xmm12, %r11
  vextracti128 $1, %ymm12, %xmm13
  vpextrq $1, %xmm12, %rbx
  vmovq %xmm13, %r14
  vmovsd (%r11), %xmm12
  vpaddq .LCPI0_5(%rip), %ymm11, %ymm14
  vmovsd (%r14), %xmm15
  vpextrq $1, %xmm14, %r11
  vpextrq $1, %xmm13, %r14
  vmovq %xmm14, %r15
  vextracti128 $1, %ymm14, %xmm13
  vmovhpd (%rbx), %xmm12, %xmm12
  vpextrq $1, %xmm13, %rbx
  vmovhpd (%r14), %xmm15, %xmm14
  vmovq %xmm13, %r14
  vmovsd (%r14), %xmm13
  vmovsd (%r15), %xmm15
  vinsertf128 $1, %xmm14, %ymm12, %ymm12
  vpaddq %ymm3, %ymm11, %ymm14
  vmovq %xmm14, %r14
  vmovhpd (%r11), %xmm15, %xmm15
  vpextrq $1, %xmm14, %r11
  vmovhpd (%rbx), %xmm13, %xmm13
  vextracti128 $1, %ymm14, %xmm14
  vpextrq $1, %xmm14, %rbx
  vinsertf128 $1, %xmm13, %ymm15, %ymm13
  vmovq %xmm14, %r15
  vmovsd (%r15), %xmm14
  vmovsd (%r14), %xmm15
  vpaddq %ymm1, %ymm11, %ymm11
  vpextrq $1, %xmm11, %r14
  vmovhpd (%r11), %xmm15, %xmm15
  vmovq %xmm11, %r11
  vextracti128 $1, %ymm11, %xmm11
  vmovhpd (%rbx), %xmm14, %xmm14
  vmovq %xmm11, %rbx
  vpextrq $1, %xmm11, %r15
  vinsertf128 $1, %xmm14, %ymm15, %ymm11
  vmovsd (%rbx), %xmm14
  vmovsd (%r11), %xmm15
  vmovhpd (%r15), %xmm14, %xmm14
  vmovhpd (%r14), %xmm15, %xmm15
  vinsertf128 $1, %xmm14, %ymm15, %ymm14
  vsubpd %ymm12, %ymm4, %ymm12
  vsubpd %ymm13, %ymm5, %ymm13
  vmulpd %ymm12, %ymm12, %ymm12
  vfmadd231pd %ymm13, %ymm13, %ymm12
  vmovdqu .LCPI0_4(%rip), %ymm13
  vsubpd %ymm11, %ymm7, %ymm11
  vfmadd231pd %ymm11, %ymm11, %ymm12
  vsqrtpd %ymm12, %ymm11
  vmulpd %ymm8, %ymm14, %ymm12
  vdivpd %ymm11, %ymm12, %ymm11
  vsubpd %ymm11, %ymm6, %ymm6
  addq $4, %r10
  cmpq %r9, %r10
  jle .LBB0_7
  vextractf128 $1, %ymm6, %xmm4
  vaddpd %xmm4, %xmm6, %xmm4
  vshufpd $1, %xmm4, %xmm4, %xmm5
  vaddsd %xmm5, %xmm4, %xmm4
  vaddsd %xmm4, %xmm0, %xmm0
  cmpq %rdi, %r8
  vmovdqa %ymm3, %ymm7
  vmovdqa %ymm1, %ymm8
  vmovupd 16(%rsp), %xmm1
  vmovupd 288(%rsp), %ymm6
  je .LBB0_9
  jmp .LBB0_4
.LBB0_3:
  xorl %edi, %edi
.LBB0_4:
  vshufpd $1, %xmm6, %xmm6, %xmm3
  imulq $56, %rdi, %r8
  addq %rax, %r8
  addq %rcx, %rdi
  imulq $56, %rdi, %rdi
  xorl %r9d, %r9d
.LBB0_5:
  vsubpd bodies+8(%rdi,%r9), %xmm2, %xmm4
  vmulpd %xmm4, %xmm4, %xmm4
  vshufpd $1, %xmm4, %xmm4, %xmm5
  vsubsd bodies(%rdi,%r9), %xmm1, %xmm6
  vaddsd %xmm4, %xmm5, %xmm4
  vfmadd213sd %xmm4, %xmm6, %xmm6
  vsqrtsd %xmm6, %xmm6, %xmm4
  vmulsd bodies+48(%rdi,%r9), %xmm3, %xmm5
  vdivsd %xmm4, %xmm5, %xmm4
  vsubsd %xmm4, %xmm0, %xmm0
  addq $56, %r9
  movq %r8, %r10
  addq %r9, %r10
  jne .LBB0_5
  jmp .LBB0_9
.LBB0_10:
  movl $.L.str, %edi
  movb $1, %al
  vzeroupper
  callq printf
  vmovupd bodies+56(%rip), %xmm1
  vmovupd bodies+16(%rip), %xmm3
  vmovupd bodies+112(%rip), %xmm0
  vmovsd bodies+128(%rip), %xmm11
  vmovupd bodies(%rip), %xmm7
  vmovupd bodies+168(%rip), %xmm5
  vmovupd bodies+224(%rip), %xmm9
  vmovupd bodies+240(%rip), %xmm4
  vmovhpd bodies+64(%rip), %xmm0, %xmm0
  vmovhpd bodies+120(%rip), %xmm1, %xmm2
  vmovhpd bodies+184(%rip), %xmm4, %xmm4
  vmovhpd bodies+72(%rip), %xmm3, %xmm14
  vmovups 64(%rsp), %xmm1
  vunpcklpd 80(%rsp), %xmm1, %xmm1
  vmovups %xmm1, 368(%rsp)
  vmovupd 320(%rsp), %xmm1
  vunpcklpd 240(%rsp), %xmm1, %xmm3
  vmovupd %xmm3, 432(%rsp)
  vmovupd 336(%rsp), %xmm3
  vunpcklpd %xmm1, %xmm3, %xmm1
  vmovupd %xmm1, 416(%rsp)
  vmovupd 352(%rsp), %xmm6
  vunpcklpd %xmm3, %xmm6, %xmm1
  vmovupd %xmm1, 400(%rsp)
  vunpcklpd %xmm6, %xmm3, %xmm1
  vmovupd %xmm1, 384(%rsp)
  movl $5000000, %eax
  vmovupd 48(%rsp), %xmm6
.LBB0_11:
  vmovupd %xmm6, 48(%rsp)
  vmovupd %xmm4, 16(%rsp)
  vmovupd %xmm5, 288(%rsp)
  vmovupd %xmm11, 672(%rsp)
  vmovupd %xmm14, 64(%rsp)
  vmovupd %xmm7, 80(%rsp)
  vmovapd %xmm2, %xmm8
  vmovapd %xmm0, %xmm10
  vsubpd %xmm2, %xmm7, %xmm6
  vsubpd %xmm0, %xmm7, %xmm1
  vmulpd %xmm6, %xmm6, %xmm0
  vmovapd %xmm6, %xmm7
  vmovupd %xmm6, 480(%rsp)
  vshufpd $1, %xmm0, %xmm0, %xmm0
  vfmadd231pd %xmm1, %xmm1, %xmm0
  vmovapd %xmm1, %xmm6
  vmovupd %xmm1, 496(%rsp)
  vmovddup %xmm14, %xmm2
  vmovddup %xmm11, %xmm1
  vblendpd $1, %xmm1, %xmm14, %xmm3
  vsubpd %xmm3, %xmm2, %xmm2
  vmovupd %xmm2, 96(%rsp)
  vfmadd231pd %xmm2, %xmm2, %xmm0
  vsqrtpd %xmm0, %xmm2
  vmulpd %xmm0, %xmm2, %xmm0
  vmovddup .LCPI0_8(%rip), %xmm12
  vdivpd %xmm0, %xmm12, %xmm3
  vmovupd %xmm3, 160(%rsp)
  vmulsd 256(%rsp), %xmm3, %xmm0
  vmovupd %xmm0, 560(%rsp)
  vmovddup %xmm0, %xmm0
  vblendpd $1, %xmm6, %xmm7, %xmm2
  vfmadd213pd 32(%rsp), %xmm0, %xmm2
  vshufpd $3, %xmm14, %xmm14, %xmm0
  vunpckhpd %xmm1, %xmm4, %xmm3
  vsubpd %xmm3, %xmm0, %xmm3
  vblendpd $1, %xmm8, %xmm10, %xmm6
  vblendpd $1, %xmm5, %xmm8, %xmm0
  vmovapd %xmm8, %xmm14
  vmovupd %xmm8, 640(%rsp)
  vsubpd %xmm0, %xmm6, %xmm15
  vblendpd $1, %xmm10, %xmm5, %xmm0
  vmovupd %xmm10, 656(%rsp)
  vsubpd %xmm0, %xmm6, %xmm11
  vunpcklpd %xmm11, %xmm15, %xmm0
  vmulpd %xmm0, %xmm0, %xmm0
  vunpckhpd %xmm15, %xmm11, %xmm4
  vfmadd213pd %xmm0, %xmm4, %xmm4
  vfmadd231pd %xmm3, %xmm3, %xmm4
  vmovapd %xmm3, %xmm8
  vmovupd %xmm3, 448(%rsp)
  vsqrtpd %xmm4, %xmm0
  vmulpd %xmm4, %xmm0, %xmm0
  vdivpd %xmm0, %xmm12, %xmm13
  vshufpd $1, %xmm13, %xmm13, %xmm0
  vmulsd 240(%rsp), %xmm0, %xmm4
  vmovddup %xmm4, %xmm0
  vblendpd $1, %xmm11, %xmm15, %xmm7
  vfmadd213pd %xmm2, %xmm0, %xmm7
  vmovupd %xmm7, 32(%rsp)
  vmovddup %xmm10, %xmm0
  vunpcklpd %xmm5, %xmm9, %xmm2
  vsubpd %xmm2, %xmm0, %xmm7
  vmovupd %xmm7, 592(%rsp)
  vshufpd $3, %xmm14, %xmm14, %xmm2
  vunpckhpd 288(%rsp), %xmm9, %xmm5
  vsubpd %xmm5, %xmm2, %xmm14
  vmovupd %xmm14, 576(%rsp)
  vsubpd 16(%rsp), %xmm1, %xmm3
  vmulpd %xmm7, %xmm7, %xmm1
  vfmadd231pd %xmm14, %xmm14, %xmm1
  vfmadd231pd %xmm3, %xmm3, %xmm1
  vmovupd %xmm3, 608(%rsp)
  vsqrtpd %xmm1, %xmm2
  vmulpd %xmm1, %xmm2, %xmm1
  vdivpd %xmm1, %xmm12, %xmm0
  vmovupd %xmm0, 512(%rsp)
  vmulpd 400(%rsp), %xmm0, %xmm1
  vshufpd $1, %xmm1, %xmm1, %xmm2
  vshufpd $1, %xmm14, %xmm7, %xmm5
  vmulpd %xmm5, %xmm2, %xmm0
  vblendpd $1, %xmm7, %xmm14, %xmm2
  vfnmsub231pd %xmm2, %xmm1, %xmm0
  vmovupd %xmm0, 624(%rsp)
  vmulpd %xmm3, %xmm1, %xmm1
  vshufpd $1, %xmm1, %xmm1, %xmm2
  vaddsd %xmm1, %xmm2, %xmm0
  vshufpd $1, %xmm8, %xmm8, %xmm1
  vfmsub231sd %xmm1, %xmm4, %xmm0
  vmovsd %xmm0, 280(%rsp)
  vmovupd 160(%rsp), %xmm5
  vshufpd $1, %xmm5, %xmm5, %xmm1
  vmulsd 256(%rsp), %xmm1, %xmm2
  vpermilpd $1, 96(%rsp), %xmm0
  vfmadd213sd 48(%rsp), %xmm2, %xmm0
  vmovupd %xmm0, 48(%rsp)
  vsubpd %xmm9, %xmm6, %xmm1
  vmovupd 80(%rsp), %xmm0
  vsubpd %xmm9, %xmm0, %xmm0
  vmovupd %xmm0, 544(%rsp)
  vmulpd %xmm0, %xmm0, %xmm3
  vmulpd %xmm1, %xmm1, %xmm4
  vunpcklpd %xmm4, %xmm3, %xmm3
  vunpckhpd %xmm1, %xmm0, %xmm4
  vmovapd %xmm1, %xmm7
  vmovupd %xmm1, 528(%rsp)
  vfmadd213pd %xmm3, %xmm4, %xmm4
  vmovddup 16(%rsp), %xmm3
  vmovupd 64(%rsp), %xmm0
  vsubpd %xmm3, %xmm0, %xmm14
  vfmadd231pd %xmm14, %xmm14, %xmm4
  vsqrtpd %xmm4, %xmm3
  vmulpd %xmm4, %xmm3, %xmm1
  vmulpd 432(%rsp), %xmm5, %xmm6
  vshufpd $1, %xmm6, %xmm6, %xmm5
  vmovupd 480(%rsp), %xmm0
  vfmsub213pd 176(%rsp), %xmm0, %xmm5
  vmovapd %xmm13, %xmm8
  vmulpd 416(%rsp), %xmm13, %xmm4
  vshufpd $1, %xmm4, %xmm4, %xmm3
  vfmsub213pd 112(%rsp), %xmm11, %xmm3
  vdivpd %xmm1, %xmm12, %xmm13
  vfmadd231pd %xmm4, %xmm15, %xmm3
  vmulpd 352(%rsp), %xmm13, %xmm1
  vmovupd %xmm1, 464(%rsp)
  vshufpd $3, %xmm1, %xmm1, %xmm10
  vfmadd231pd %xmm10, %xmm7, %xmm3
  vmovddup %xmm2, %xmm2
  vmovupd 496(%rsp), %xmm1
  vblendpd $1, %xmm0, %xmm1, %xmm10
  vfmsub231pd %xmm10, %xmm2, %xmm3
  vmovupd %xmm3, 112(%rsp)
  vmovupd 80(%rsp), %xmm2
  vsubpd 288(%rsp), %xmm2, %xmm10
  vmulpd %xmm10, %xmm10, %xmm2
  vshufpd $1, %xmm2, %xmm2, %xmm3
  vaddsd %xmm2, %xmm3, %xmm2
  vpermilpd $1, 16(%rsp), %xmm0
  vmovupd %xmm0, 160(%rsp)
  vmovupd 64(%rsp), %xmm3
  vsubsd %xmm0, %xmm3, %xmm3
  vfmadd231sd %xmm3, %xmm3, %xmm2
  vsqrtsd %xmm2, %xmm2, %xmm12
  vmulsd %xmm2, %xmm12, %xmm2
  vmovsd 128(%rsp), %xmm12
  vmovupd 96(%rsp), %xmm0
  vfmadd231sd 560(%rsp), %xmm0, %xmm12
  vmovsd %xmm12, 128(%rsp)
  vfmadd231pd %xmm6, %xmm1, %xmm5
  vmulpd %xmm0, %xmm6, %xmm1
  vshufpd $1, %xmm1, %xmm1, %xmm12
  vaddpd %xmm1, %xmm12, %xmm1
  vmovupd 448(%rsp), %xmm6
  vmulpd %xmm6, %xmm4, %xmm12
  vblendpd $1, %xmm1, %xmm12, %xmm1
  vmovsd .LCPI0_8(%rip), %xmm12
  vdivsd %xmm2, %xmm12, %xmm12
  vmulsd 336(%rsp), %xmm12, %xmm2
  vmovddup %xmm2, %xmm2
  vunpcklpd %xmm4, %xmm2, %xmm0
  vunpcklpd %xmm6, %xmm3, %xmm4
  vfmadd213pd %xmm1, %xmm0, %xmm4
  vmovupd %xmm4, 96(%rsp)
  vfmadd231pd %xmm2, %xmm10, %xmm5
  vunpcklpd %xmm12, %xmm13, %xmm0
  vmulpd 256(%rsp), %xmm0, %xmm0
  vshufpd $3, %xmm0, %xmm0, %xmm1
  vfmadd213pd 224(%rsp), %xmm10, %xmm1
  vshufpd $1, %xmm8, %xmm13, %xmm2
  vblendpd $1, %xmm15, %xmm11, %xmm7
  vmulpd 240(%rsp), %xmm2, %xmm2
  vshufpd $3, %xmm2, %xmm2, %xmm10
  vfmadd213pd %xmm1, %xmm10, %xmm7
  vmovupd 512(%rsp), %xmm1
  vmulpd 320(%rsp), %xmm1, %xmm10
  vshufpd $3, %xmm10, %xmm10, %xmm1
  vmovupd 592(%rsp), %xmm8
  vmovupd 576(%rsp), %xmm13
  vunpckhpd %xmm13, %xmm8, %xmm15
  vfmadd213pd %xmm7, %xmm1, %xmm15
  vmovapd %xmm5, %xmm12
  vmovupd 464(%rsp), %xmm11
  vmovddup %xmm11, %xmm1
  vmovupd 544(%rsp), %xmm7
  vfnmsub231pd %xmm1, %xmm7, %xmm12
  vunpcklpd %xmm3, %xmm14, %xmm3
  vfmadd213pd 368(%rsp), %xmm0, %xmm3
  vmovddup %xmm0, %xmm5
  vfmadd213pd 192(%rsp), %xmm7, %xmm5
  vmovupd 208(%rsp), %xmm0
  vunpcklpd 48(%rsp), %xmm0, %xmm0
  vfmsub231pd %xmm11, %xmm14, %xmm0
  vmovupd %xmm12, 176(%rsp)
  vshufpd $1, %xmm6, %xmm14, %xmm1
  vmovupd 672(%rsp), %xmm11
  vfmadd213pd %xmm3, %xmm2, %xmm1
  vfmadd231pd 608(%rsp), %xmm10, %xmm1
  vmulpd %xmm8, %xmm10, %xmm3
  vmulpd %xmm13, %xmm10, %xmm4
  vmovupd 64(%rsp), %xmm14
  vunpcklpd %xmm4, %xmm3, %xmm3
  vmovddup %xmm2, %xmm4
  vfmadd132pd 528(%rsp), %xmm3, %xmm4
  vmovupd 288(%rsp), %xmm2
  vsubpd %xmm9, %xmm2, %xmm2
  vmulpd %xmm2, %xmm2, %xmm3
  vshufpd $1, %xmm3, %xmm3, %xmm6
  vaddsd %xmm3, %xmm6, %xmm3
  vmovupd 160(%rsp), %xmm6
  vsubsd 16(%rsp), %xmm6, %xmm6
  vfmadd231sd %xmm6, %xmm6, %xmm3
  vsqrtsd %xmm3, %xmm3, %xmm7
  vmulsd %xmm3, %xmm7, %xmm3
  vmovsd .LCPI0_8(%rip), %xmm13
  vdivsd %xmm3, %xmm13, %xmm3
  vmovddup %xmm3, %xmm3
  vmulpd 384(%rsp), %xmm3, %xmm7
  vshufpd $3, %xmm7, %xmm7, %xmm3
  vfnmadd231pd %xmm3, %xmm2, %xmm15
  vmovddup %xmm6, %xmm6
  vmovapd %xmm7, %xmm3
  vfmadd213pd %xmm1, %xmm6, %xmm3
  vfnmadd231pd %xmm7, %xmm6, %xmm1
  vmovddup %xmm7, %xmm6
  vmovupd 80(%rsp), %xmm7
  vfmadd231pd %xmm6, %xmm2, %xmm5
  vmovupd 624(%rsp), %xmm2
  vaddpd 32(%rsp), %xmm2, %xmm6
  vmovsd 128(%rsp), %xmm2
  vaddsd 280(%rsp), %xmm2, %xmm2
  vmovsd %xmm2, 128(%rsp)
  vaddpd 96(%rsp), %xmm0, %xmm0
  vxorpd %xmm2, %xmm2, %xmm2
  vsubpd %xmm0, %xmm2, %xmm8
  vmovddup .LCPI0_8(%rip), %xmm10
  vfmadd231pd %xmm10, %xmm12, %xmm7
  vmovupd 112(%rsp), %xmm2
  vblendpd $1, %xmm6, %xmm2, %xmm0
  vfmadd213pd 656(%rsp), %xmm10, %xmm0
  vmovupd %xmm6, 32(%rsp)
  vblendpd $1, %xmm2, %xmm6, %xmm2
  vfmadd231pd %xmm10, %xmm8, %xmm14
  vfmadd213pd 640(%rsp), %xmm10, %xmm2
  vmovapd %xmm10, %xmm6
  vfmadd231sd 128(%rsp), %xmm13, %xmm11
  vblendpd $1, %xmm3, %xmm1, %xmm13
  vaddpd %xmm4, %xmm5, %xmm10
  vmovupd 16(%rsp), %xmm4
  vmovupd 288(%rsp), %xmm5
  vmovupd %xmm15, 224(%rsp)
  vfmadd231pd %xmm6, %xmm15, %xmm5
  vmovupd %xmm10, 192(%rsp)
  vfmadd231pd %xmm6, %xmm10, %xmm9
  vmovupd %xmm13, 368(%rsp)
  vfmadd231pd %xmm6, %xmm13, %xmm4
  vmovupd %xmm8, 208(%rsp)
  vshufpd $1, %xmm8, %xmm8, %xmm6
  decl %eax
  jne .LBB0_11
  vmovlpd %xmm3, bodies+264(%rip)
  vmovupd 192(%rsp), %xmm3
  vmovupd %xmm3, bodies+248(%rip)
  vmovlpd %xmm4, bodies+240(%rip)
  vmovupd %xmm9, bodies+224(%rip)
  vmovhpd %xmm1, bodies+208(%rip)
  vmovups 224(%rsp), %xmm1
  vmovups %xmm1, bodies+192(%rip)
  vmovhpd %xmm4, bodies+184(%rip)
  vmovupd %xmm5, bodies+168(%rip)
  vmovsd 128(%rsp), %xmm1
  vmovsd %xmm1, bodies+152(%rip)
  vmovups 32(%rsp), %xmm1
  vmovups %xmm1, bodies+136(%rip)
  vmovsd %xmm11, bodies+128(%rip)
  vblendpd $1, %xmm0, %xmm2, %xmm1
  vmovupd %xmm1, bodies+112(%rip)
  vmovsd %xmm6, bodies+96(%rip)
  vmovups 112(%rsp), %xmm1
  vmovups %xmm1, bodies+80(%rip)
  vmovups 208(%rsp), %xmm1
  vmovsd %xmm1, bodies+40(%rip)
  vmovupd 176(%rsp), %xmm1
  vmovupd %xmm1, bodies+24(%rip)
  vblendpd $1, %xmm2, %xmm0, %xmm0
  vmovhpd %xmm14, bodies+72(%rip)
  vmovupd %xmm0, bodies+56(%rip)
  vmovlpd %xmm14, bodies+16(%rip)
  vmovupd %xmm7, bodies(%rip)
  vxorpd %xmm0, %xmm0, %xmm0
  movq $-224, %rax
  movl $1, %ecx
  xorl %esi, %esi
  movl $bodies, %edx
  vpbroadcastq .LCPI0_3(%rip), %ymm9
  jmp .LBB0_13
.LBB0_21:
  leaq 1(%rsi), %rdi
  addq $56, %rax
  incq %rcx
  cmpq $4, %rsi
  movq %rdi, %rsi
  je .LBB0_22
.LBB0_13:
  vmovapd %xmm0, %xmm1
  imulq $56, %rsi, %rdi
  vmovsd bodies+24(%rdi), %xmm2
  vmovsd bodies+32(%rdi), %xmm0
  vmovupd bodies+40(%rdi), %xmm3
  vmovhpd .LCPI0_2(%rip), %xmm3, %xmm4
  vmulpd %xmm4, %xmm3, %xmm4
  vfmadd213sd %xmm4, %xmm2, %xmm2
  vfmadd231sd %xmm0, %xmm0, %xmm2
  vshufpd $1, %xmm4, %xmm4, %xmm0
  vfmadd213sd %xmm1, %xmm2, %xmm0
  cmpq $3, %rsi
  ja .LBB0_21
  vmovsd bodies(%rdi), %xmm1
  vmovupd bodies+8(%rdi), %xmm2
  movl $4, %r8d
  subq %rsi, %r8
  movq %r8, %rdi
  andq $-4, %rdi
  je .LBB0_15
  leaq -1(%rdi), %r9
  vmovupd %xmm1, 16(%rsp)
  vbroadcastsd %xmm1, %ymm4
  vbroadcastsd %xmm2, %ymm5
  vpermpd $85, %ymm2, %ymm7
  vmovupd %ymm3, 128(%rsp)
  vpermpd $85, %ymm3, %ymm8
  vxorpd %xmm6, %xmm6, %xmm6
  vmovq %rdx, %xmm10
  vpbroadcastq %xmm10, %ymm10
  xorl %r10d, %r10d
  vmovdqu .LCPI0_6(%rip), %ymm3
  vmovdqu .LCPI0_7(%rip), %ymm1
.LBB0_19:
  leaq (%rsi,%r10), %r11
  vmovq %r11, %xmm11
  vpbroadcastq %xmm11, %ymm11
  vpmuludq %ymm9, %ymm11, %ymm12
  vpsrlq $32, %ymm11, %ymm11
  vpaddq %ymm10, %ymm12, %ymm12
  vpmuludq %ymm9, %ymm11, %ymm11
  vpsllq $32, %ymm11, %ymm11
  vpaddq %ymm11, %ymm12, %ymm11
  vpaddq .LCPI0_4(%rip), %ymm11, %ymm12
  vmovq %xmm12, %r11
  vextracti128 $1, %ymm12, %xmm13
  vpextrq $1, %xmm12, %rbx
  vmovq %xmm13, %r14
  vmovsd (%r11), %xmm12
  vpaddq .LCPI0_5(%rip), %ymm11, %ymm14
  vmovsd (%r14), %xmm15
  vpextrq $1, %xmm14, %r11
  vpextrq $1, %xmm13, %r14
  vmovq %xmm14, %r15
  vextracti128 $1, %ymm14, %xmm13
  vmovhpd (%rbx), %xmm12, %xmm12
  vpextrq $1, %xmm13, %rbx
  vmovhpd (%r14), %xmm15, %xmm14
  vmovq %xmm13, %r14
  vmovsd (%r14), %xmm13
  vmovsd (%r15), %xmm15
  vinsertf128 $1, %xmm14, %ymm12, %ymm12
  vpaddq %ymm3, %ymm11, %ymm14
  vmovq %xmm14, %r14
  vmovhpd (%r11), %xmm15, %xmm15
  vpextrq $1, %xmm14, %r11
  vmovhpd (%rbx), %xmm13, %xmm13
  vextracti128 $1, %ymm14, %xmm14
  vpextrq $1, %xmm14, %rbx
  vinsertf128 $1, %xmm13, %ymm15, %ymm13
  vmovq %xmm14, %r15
  vmovsd (%r15), %xmm14
  vmovsd (%r14), %xmm15
  vpaddq %ymm1, %ymm11, %ymm11
  vpextrq $1, %xmm11, %r14
  vmovhpd (%r11), %xmm15, %xmm15
  vmovq %xmm11, %r11
  vextracti128 $1, %ymm11, %xmm11
  vmovhpd (%rbx), %xmm14, %xmm14
  vmovq %xmm11, %rbx
  vpextrq $1, %xmm11, %r15
  vinsertf128 $1, %xmm14, %ymm15, %ymm11
  vmovsd (%rbx), %xmm14
  vmovsd (%r11), %xmm15
  vmovhpd (%r15), %xmm14, %xmm14
  vmovhpd (%r14), %xmm15, %xmm15
  vinsertf128 $1, %xmm14, %ymm15, %ymm14
  vsubpd %ymm12, %ymm4, %ymm12
  vsubpd %ymm13, %ymm5, %ymm13
  vmulpd %ymm12, %ymm12, %ymm12
  vfmadd231pd %ymm13, %ymm13, %ymm12
  vsubpd %ymm11, %ymm7, %ymm11
  vfmadd231pd %ymm11, %ymm11, %ymm12
  vsqrtpd %ymm12, %ymm11
  vmulpd %ymm8, %ymm14, %ymm12
  vdivpd %ymm11, %ymm12, %ymm11
  vsubpd %ymm11, %ymm6, %ymm6
  addq $4, %r10
  cmpq %r9, %r10
  jle .LBB0_19
  vextractf128 $1, %ymm6, %xmm4
  vaddpd %xmm4, %xmm6, %xmm4
  vshufpd $1, %xmm4, %xmm4, %xmm5
  vaddsd %xmm5, %xmm4, %xmm4
  vaddsd %xmm4, %xmm0, %xmm0
  cmpq %rdi, %r8
  vmovupd 16(%rsp), %xmm1
  vmovupd 128(%rsp), %ymm3
  je .LBB0_21
  jmp .LBB0_16
.LBB0_15:
  xorl %edi, %edi
.LBB0_16:
  vshufpd $1, %xmm3, %xmm3, %xmm3
  imulq $56, %rdi, %r8
  addq %rax, %r8
  addq %rcx, %rdi
  imulq $56, %rdi, %rdi
  xorl %r9d, %r9d
.LBB0_17:
  vsubpd bodies+8(%rdi,%r9), %xmm2, %xmm4
  vmulpd %xmm4, %xmm4, %xmm4
  vshufpd $1, %xmm4, %xmm4, %xmm5
  vsubsd bodies(%rdi,%r9), %xmm1, %xmm6
  vaddsd %xmm4, %xmm5, %xmm4
  vfmadd213sd %xmm4, %xmm6, %xmm6
  vsqrtsd %xmm6, %xmm6, %xmm4
  vmulsd bodies+48(%rdi,%r9), %xmm3, %xmm5
  vdivsd %xmm4, %xmm5, %xmm4
  vsubsd %xmm4, %xmm0, %xmm0
  addq $56, %r9
  movq %r8, %r10
  addq %r9, %r10
  jne .LBB0_17
  jmp .LBB0_21
.LBB0_22:
  movl $.L.str, %edi
  movb $1, %al
  vzeroupper
  callq printf
  xorl %eax, %eax
  addq $688, %rsp
  popq %rbx
  popq %r14
  popq %r15
  retq

bodies:

.L.str:

