.LCPI0_0:
  .quad 0xbf99f02f6222c720
.LCPI0_2:
  .quad 0x3f847ae147ae147b
.LCPI0_1:
  .quad 0xbf99f02f6222c720
  .quad 0xbf99f02f6222c720
main:
  subq $648, %rsp
  vstmxcsr 12(%rsp)
  orl $32832, 12(%rsp)
  vldmxcsr 12(%rsp)
  vmovddup bodies+48(%rip), %xmm2
  vmulsd bodies+40(%rip), %xmm2, %xmm0
  vmovddup bodies+104(%rip), %xmm1
  vmovsd bodies+96(%rip), %xmm3
  vmovupd %xmm3, 64(%rsp)
  vfmadd231sd %xmm1, %xmm3, %xmm0
  vmovapd %xmm1, %xmm3
  vmovupd %xmm1, 208(%rsp)
  vmovsd bodies+152(%rip), %xmm1
  vmovddup bodies+160(%rip), %xmm4
  vmovsd %xmm1, 40(%rsp)
  vfmadd231sd %xmm4, %xmm1, %xmm0
  vmovupd %xmm4, 320(%rsp)
  vmovsd bodies+208(%rip), %xmm5
  vmovupd %xmm5, 16(%rsp)
  vmovddup bodies+216(%rip), %xmm1
  vfmadd231sd %xmm1, %xmm5, %xmm0
  vmovapd %xmm1, %xmm5
  vmovupd %xmm1, 304(%rsp)
  vmovsd bodies+264(%rip), %xmm6
  vmovupd %xmm6, 48(%rsp)
  vmovddup bodies+272(%rip), %xmm1
  vfmadd231sd %xmm1, %xmm6, %xmm0
  vmovapd %xmm1, %xmm6
  vmovupd %xmm1, 288(%rsp)
  vmulsd .LCPI0_0(%rip), %xmm0, %xmm0
  vmovupd %xmm0, 192(%rsp)
  vmovsd %xmm0, bodies+40(%rip)
  vmovupd %xmm2, 224(%rsp)
  vmulpd bodies+24(%rip), %xmm2, %xmm0
  vmovupd bodies+80(%rip), %xmm1
  vmovupd %xmm1, 128(%rsp)
  vfmadd231pd %xmm3, %xmm1, %xmm0
  vmovupd bodies+136(%rip), %xmm1
  vmovupd %xmm1, 96(%rsp)
  vfmadd231pd %xmm4, %xmm1, %xmm0
  vmovupd bodies+192(%rip), %xmm1
  vmovupd %xmm1, 176(%rsp)
  vfmadd231pd %xmm5, %xmm1, %xmm0
  vmovupd bodies+248(%rip), %xmm1
  vmovupd %xmm1, 160(%rsp)
  vfmadd231pd %xmm6, %xmm1, %xmm0
  vmulpd .LCPI0_1(%rip), %xmm0, %xmm0
  vmovupd %xmm0, 80(%rsp)
  vmovupd %xmm0, bodies+24(%rip)
  callq energy
  movl $.L.str, %edi
  movb $1, %al
  callq printf
  vmovupd 96(%rsp), %xmm15
  vmovupd bodies(%rip), %xmm7
  vmovupd bodies+56(%rip), %xmm1
  vmovupd bodies+16(%rip), %xmm3
  vmovupd bodies+112(%rip), %xmm0
  vmovsd bodies+128(%rip), %xmm8
  vmovupd bodies+168(%rip), %xmm5
  vmovupd bodies+224(%rip), %xmm11
  vmovupd bodies+240(%rip), %xmm4
  vmovhpd bodies+64(%rip), %xmm0, %xmm0
  vmovhpd bodies+120(%rip), %xmm1, %xmm2
  vmovhpd bodies+184(%rip), %xmm4, %xmm4
  vmovhpd bodies+72(%rip), %xmm3, %xmm9
  vmovups 48(%rsp), %xmm1
  vunpcklpd 16(%rsp), %xmm1, %xmm1
  vmovups %xmm1, 336(%rsp)
  vmovupd 320(%rsp), %xmm3
  vunpcklpd 208(%rsp), %xmm3, %xmm1
  vmovupd %xmm1, 416(%rsp)
  vmovupd 304(%rsp), %xmm6
  vblendpd $1, %xmm6, %xmm3, %xmm1
  vmovupd %xmm1, 400(%rsp)
  vmovupd 288(%rsp), %xmm3
  vblendpd $1, %xmm3, %xmm6, %xmm1
  vmovupd %xmm1, 384(%rsp)
  vblendpd $1, %xmm6, %xmm3, %xmm1
  vmovupd %xmm1, 368(%rsp)
  movl $5000000, %eax
.LBB0_1:
  vmovupd %xmm4, 16(%rsp)
  vmovupd %xmm11, 352(%rsp)
  vmovupd %xmm5, 48(%rsp)
  vmovupd %xmm8, 240(%rsp)
  vmovupd %xmm9, 256(%rsp)
  vmovupd %xmm7, 96(%rsp)
  vmovapd %xmm2, %xmm6
  vmovapd %xmm0, %xmm13
  vsubpd %xmm2, %xmm7, %xmm12
  vsubpd %xmm0, %xmm7, %xmm1
  vmulpd %xmm12, %xmm12, %xmm0
  vmovupd %xmm12, 464(%rsp)
  vshufpd $1, %xmm0, %xmm0, %xmm0
  vfmadd231pd %xmm1, %xmm1, %xmm0
  vmovapd %xmm1, %xmm7
  vmovupd %xmm1, 480(%rsp)
  vmovddup %xmm9, %xmm2
  vmovddup %xmm8, %xmm1
  vblendpd $1, %xmm1, %xmm9, %xmm3
  vsubpd %xmm3, %xmm2, %xmm2
  vmovupd %xmm2, 112(%rsp)
  vfmadd231pd %xmm2, %xmm2, %xmm0
  vsqrtpd %xmm0, %xmm2
  vmulpd %xmm0, %xmm2, %xmm0
  vmovddup .LCPI0_2(%rip), %xmm2
  vdivpd %xmm0, %xmm2, %xmm3
  vmovupd %xmm3, 144(%rsp)
  vmovapd %xmm2, %xmm8
  vmulsd 224(%rsp), %xmm3, %xmm2
  vmovupd %xmm2, 544(%rsp)
  vmovddup %xmm2, %xmm0
  vblendpd $1, %xmm7, %xmm12, %xmm2
  vfmadd213pd %xmm15, %xmm0, %xmm2
  vshufpd $3, %xmm9, %xmm9, %xmm0
  vunpckhpd %xmm1, %xmm4, %xmm3
  vsubpd %xmm3, %xmm0, %xmm10
  vblendpd $1, %xmm6, %xmm13, %xmm3
  vblendpd $1, %xmm5, %xmm6, %xmm0
  vmovapd %xmm6, %xmm9
  vmovupd %xmm6, 608(%rsp)
  vsubpd %xmm0, %xmm3, %xmm15
  vblendpd $1, %xmm13, %xmm5, %xmm0
  vmovupd %xmm13, 624(%rsp)
  vsubpd %xmm0, %xmm3, %xmm7
  vunpcklpd %xmm7, %xmm15, %xmm0
  vmulpd %xmm0, %xmm0, %xmm0
  vunpckhpd %xmm15, %xmm7, %xmm4
  vfmadd213pd %xmm0, %xmm4, %xmm4
  vfmadd231pd %xmm10, %xmm10, %xmm4
  vmovupd %xmm10, 448(%rsp)
  vsqrtpd %xmm4, %xmm0
  vmulpd %xmm4, %xmm0, %xmm0
  vdivpd %xmm0, %xmm8, %xmm12
  vshufpd $1, %xmm12, %xmm12, %xmm0
  vmovupd %xmm12, 432(%rsp)
  vmulsd 208(%rsp), %xmm0, %xmm4
  vmovddup %xmm4, %xmm0
  vblendpd $1, %xmm7, %xmm15, %xmm6
  vfmadd213pd %xmm2, %xmm0, %xmm6
  vmovupd %xmm6, 592(%rsp)
  vmovddup %xmm13, %xmm0
  vunpcklpd %xmm5, %xmm11, %xmm2
  vsubpd %xmm2, %xmm0, %xmm14
  vshufpd $3, %xmm9, %xmm9, %xmm2
  vunpckhpd 48(%rsp), %xmm11, %xmm5
  vsubpd %xmm5, %xmm2, %xmm9
  vsubpd 16(%rsp), %xmm1, %xmm13
  vmovupd %xmm13, 560(%rsp)
  vmulpd %xmm14, %xmm14, %xmm1
  vfmadd231pd %xmm9, %xmm9, %xmm1
  vfmadd231pd %xmm13, %xmm13, %xmm1
  vsqrtpd %xmm1, %xmm2
  vmulpd %xmm1, %xmm2, %xmm1
  vdivpd %xmm1, %xmm8, %xmm0
  vmovupd %xmm0, 528(%rsp)
  vmulpd 384(%rsp), %xmm0, %xmm1
  vshufpd $1, %xmm1, %xmm1, %xmm2
  vshufpd $1, %xmm9, %xmm14, %xmm5
  vmulpd %xmm5, %xmm2, %xmm0
  vblendpd $1, %xmm14, %xmm9, %xmm2
  vfnmsub231pd %xmm2, %xmm1, %xmm0
  vmovupd %xmm0, 576(%rsp)
  vmulpd %xmm1, %xmm13, %xmm1
  vshufpd $1, %xmm1, %xmm1, %xmm2
  vaddsd %xmm1, %xmm2, %xmm0
  vshufpd $1, %xmm10, %xmm10, %xmm1
  vfmsub231sd %xmm1, %xmm4, %xmm0
  vmovsd %xmm0, 280(%rsp)
  vmovupd 144(%rsp), %xmm5
  vshufpd $1, %xmm5, %xmm5, %xmm1
  vmulsd 224(%rsp), %xmm1, %xmm2
  vpermilpd $1, 112(%rsp), %xmm0
  vfmadd213sd 64(%rsp), %xmm2, %xmm0
  vmovupd %xmm0, 64(%rsp)
  vsubpd %xmm11, %xmm3, %xmm1
  vmovupd 96(%rsp), %xmm0
  vsubpd %xmm11, %xmm0, %xmm0
  vmovupd %xmm0, 512(%rsp)
  vmulpd %xmm0, %xmm0, %xmm3
  vmulpd %xmm1, %xmm1, %xmm4
  vunpcklpd %xmm4, %xmm3, %xmm3
  vunpckhpd %xmm1, %xmm0, %xmm4
  vmovupd %xmm1, 496(%rsp)
  vfmadd213pd %xmm3, %xmm4, %xmm4
  vmovddup 16(%rsp), %xmm3
  vmovupd 256(%rsp), %xmm0
  vsubpd %xmm3, %xmm0, %xmm11
  vfmadd231pd %xmm11, %xmm11, %xmm4
  vsqrtpd %xmm4, %xmm3
  vmulpd %xmm4, %xmm3, %xmm13
  vmulpd 416(%rsp), %xmm5, %xmm4
  vshufpd $1, %xmm4, %xmm4, %xmm5
  vmovupd 464(%rsp), %xmm8
  vfmsub213pd 80(%rsp), %xmm8, %xmm5
  vmulpd 400(%rsp), %xmm12, %xmm0
  vshufpd $1, %xmm0, %xmm0, %xmm3
  vfmsub213pd 128(%rsp), %xmm7, %xmm3
  vmovddup .LCPI0_2(%rip), %xmm10
  vdivpd %xmm13, %xmm10, %xmm13
  vfmadd231pd %xmm0, %xmm15, %xmm3
  vmulpd 288(%rsp), %xmm13, %xmm6
  vmovupd %xmm6, 144(%rsp)
  vshufpd $3, %xmm6, %xmm6, %xmm10
  vfmadd231pd %xmm10, %xmm1, %xmm3
  vmovddup %xmm2, %xmm2
  vmovupd 480(%rsp), %xmm1
  vblendpd $1, %xmm8, %xmm1, %xmm10
  vfmsub231pd %xmm10, %xmm2, %xmm3
  vmovupd %xmm3, 128(%rsp)
  vmovupd 96(%rsp), %xmm2
  vsubpd 48(%rsp), %xmm2, %xmm10
  vmulpd %xmm10, %xmm10, %xmm2
  vshufpd $1, %xmm2, %xmm2, %xmm3
  vaddsd %xmm2, %xmm3, %xmm2
  vpermilpd $1, 16(%rsp), %xmm6
  vmovupd %xmm6, 80(%rsp)
  vmovupd 256(%rsp), %xmm3
  vsubsd %xmm6, %xmm3, %xmm3
  vfmadd231sd %xmm3, %xmm3, %xmm2
  vsqrtsd %xmm2, %xmm2, %xmm12
  vmulsd %xmm2, %xmm12, %xmm2
  vmovsd 40(%rsp), %xmm8
  vmovupd 112(%rsp), %xmm12
  vfmadd231sd 544(%rsp), %xmm12, %xmm8
  vfmadd231pd %xmm4, %xmm1, %xmm5
  vmulpd %xmm4, %xmm12, %xmm1
  vshufpd $1, %xmm1, %xmm1, %xmm12
  vaddpd %xmm1, %xmm12, %xmm1
  vmovupd 448(%rsp), %xmm4
  vmulpd %xmm4, %xmm0, %xmm12
  vblendpd $1, %xmm1, %xmm12, %xmm1
  vmovsd .LCPI0_2(%rip), %xmm12
  vdivsd %xmm2, %xmm12, %xmm12
  vmulsd 304(%rsp), %xmm12, %xmm2
  vmovddup %xmm2, %xmm2
  vunpcklpd %xmm0, %xmm2, %xmm0
  vunpcklpd %xmm4, %xmm3, %xmm6
  vfmadd213pd %xmm1, %xmm0, %xmm6
  vmovupd %xmm6, 112(%rsp)
  vfmadd231pd %xmm2, %xmm10, %xmm5
  vunpcklpd %xmm12, %xmm13, %xmm0
  vmulpd 224(%rsp), %xmm0, %xmm0
  vshufpd $3, %xmm0, %xmm0, %xmm1
  vfmadd213pd 176(%rsp), %xmm10, %xmm1
  vshufpd $1, 432(%rsp), %xmm13, %xmm2
  vblendpd $1, %xmm15, %xmm7, %xmm7
  vmulpd 208(%rsp), %xmm2, %xmm2
  vshufpd $3, %xmm2, %xmm2, %xmm10
  vfmadd213pd %xmm1, %xmm10, %xmm7
  vmovupd 528(%rsp), %xmm1
  vmulpd 320(%rsp), %xmm1, %xmm10
  vshufpd $3, %xmm10, %xmm10, %xmm1
  vunpckhpd %xmm9, %xmm14, %xmm12
  vfmadd213pd %xmm7, %xmm1, %xmm12
  vmovapd %xmm5, %xmm13
  vmovupd 144(%rsp), %xmm7
  vmovddup %xmm7, %xmm1
  vmovupd 512(%rsp), %xmm6
  vfnmsub231pd %xmm1, %xmm6, %xmm13
  vunpcklpd %xmm3, %xmm11, %xmm3
  vfmadd213pd 336(%rsp), %xmm0, %xmm3
  vmovddup %xmm0, %xmm5
  vfmadd213pd 160(%rsp), %xmm6, %xmm5
  vmovupd 192(%rsp), %xmm0
  vunpcklpd 64(%rsp), %xmm0, %xmm0
  vfmsub231pd %xmm7, %xmm11, %xmm0
  vshufpd $1, %xmm4, %xmm11, %xmm1
  vfmadd213pd %xmm3, %xmm2, %xmm1
  vfmadd231pd 560(%rsp), %xmm10, %xmm1
  vmulpd %xmm14, %xmm10, %xmm3
  vmulpd %xmm9, %xmm10, %xmm4
  vmovupd 256(%rsp), %xmm9
  vmovupd 128(%rsp), %xmm10
  vunpcklpd %xmm4, %xmm3, %xmm3
  vmovddup %xmm2, %xmm4
  vfmadd132pd 496(%rsp), %xmm3, %xmm4
  vmovupd 48(%rsp), %xmm2
  vsubpd 352(%rsp), %xmm2, %xmm2
  vmulpd %xmm2, %xmm2, %xmm3
  vshufpd $1, %xmm3, %xmm3, %xmm6
  vaddsd %xmm3, %xmm6, %xmm3
  vmovupd 80(%rsp), %xmm6
  vsubsd 16(%rsp), %xmm6, %xmm6
  vfmadd231sd %xmm6, %xmm6, %xmm3
  vsqrtsd %xmm3, %xmm3, %xmm7
  vmulsd %xmm3, %xmm7, %xmm3
  vmovsd .LCPI0_2(%rip), %xmm14
  vdivsd %xmm3, %xmm14, %xmm3
  vmovddup %xmm3, %xmm3
  vmulpd 368(%rsp), %xmm3, %xmm7
  vshufpd $3, %xmm7, %xmm7, %xmm3
  vfnmadd231pd %xmm3, %xmm2, %xmm12
  vmovddup %xmm6, %xmm6
  vmovapd %xmm7, %xmm3
  vfmadd213pd %xmm1, %xmm6, %xmm3
  vfnmadd231pd %xmm7, %xmm6, %xmm1
  vmovddup %xmm7, %xmm6
  vmovupd 96(%rsp), %xmm7
  vfmadd231pd %xmm6, %xmm2, %xmm5
  vmovupd 576(%rsp), %xmm2
  vaddpd 592(%rsp), %xmm2, %xmm15
  vaddsd 280(%rsp), %xmm8, %xmm8
  vaddpd 112(%rsp), %xmm0, %xmm0
  vxorpd %xmm2, %xmm2, %xmm2
  vsubpd %xmm0, %xmm2, %xmm6
  vmovupd %xmm13, 80(%rsp)
  vmovddup .LCPI0_2(%rip), %xmm11
  vfmadd231pd %xmm11, %xmm13, %xmm7
  vblendpd $1, %xmm15, %xmm10, %xmm0
  vfmadd213pd 624(%rsp), %xmm11, %xmm0
  vblendpd $1, %xmm10, %xmm15, %xmm2
  vfmadd231pd %xmm11, %xmm6, %xmm9
  vfmadd213pd 608(%rsp), %xmm11, %xmm2
  vmovapd %xmm11, %xmm10
  vmovsd %xmm8, 40(%rsp)
  vmovupd 240(%rsp), %xmm11
  vfmadd231sd %xmm14, %xmm8, %xmm11
  vmovupd %xmm11, 240(%rsp)
  vmovupd 352(%rsp), %xmm11
  vmovupd 240(%rsp), %xmm8
  vblendpd $1, %xmm3, %xmm1, %xmm14
  vaddpd %xmm4, %xmm5, %xmm13
  vmovupd 16(%rsp), %xmm4
  vmovupd 48(%rsp), %xmm5
  vmovupd %xmm12, 176(%rsp)
  vfmadd231pd %xmm10, %xmm12, %xmm5
  vmovupd %xmm13, 160(%rsp)
  vfmadd231pd %xmm10, %xmm13, %xmm11
  vmovupd %xmm14, 336(%rsp)
  vfmadd231pd %xmm10, %xmm14, %xmm4
  vmovupd %xmm6, 192(%rsp)
  vshufpd $1, %xmm6, %xmm6, %xmm6
  vmovupd %xmm6, 64(%rsp)
  decl %eax
  jne .LBB0_1
  vmovlpd %xmm3, bodies+264(%rip)
  vmovups 160(%rsp), %xmm3
  vmovups %xmm3, bodies+248(%rip)
  vmovlpd %xmm4, bodies+240(%rip)
  vmovupd %xmm11, bodies+224(%rip)
  vmovhpd %xmm1, bodies+208(%rip)
  vmovups 176(%rsp), %xmm1
  vmovups %xmm1, bodies+192(%rip)
  vmovhpd %xmm4, bodies+184(%rip)
  vmovupd %xmm5, bodies+168(%rip)
  vmovsd 40(%rsp), %xmm1
  vmovsd %xmm1, bodies+152(%rip)
  vmovupd %xmm15, bodies+136(%rip)
  vmovsd %xmm8, bodies+128(%rip)
  vblendpd $1, %xmm0, %xmm2, %xmm1
  vmovupd %xmm1, bodies+112(%rip)
  vmovups 64(%rsp), %xmm1
  vmovsd %xmm1, bodies+96(%rip)
  vmovups 128(%rsp), %xmm1
  vmovups %xmm1, bodies+80(%rip)
  vmovups 192(%rsp), %xmm1
  vmovsd %xmm1, bodies+40(%rip)
  vmovups 80(%rsp), %xmm1
  vmovups %xmm1, bodies+24(%rip)
  vblendpd $1, %xmm2, %xmm0, %xmm0
  vmovhpd %xmm9, bodies+72(%rip)
  vmovupd %xmm0, bodies+56(%rip)
  vmovlpd %xmm9, bodies+16(%rip)
  vmovupd %xmm7, bodies(%rip)
  callq energy
  movl $.L.str, %edi
  movb $1, %al
  callq printf
  xorl %eax, %eax
  addq $648, %rsp
  retq

.LCPI1_0:
  .quad 0x3fe0000000000000
.LCPI1_1:
  .quad 56
.LCPI1_2:
  .quad 56
  .quad 112
  .quad 168
  .quad 224
.LCPI1_3:
  .quad 64
  .quad 120
  .quad 176
  .quad 232
.LCPI1_4:
  .quad 72
  .quad 128
  .quad 184
  .quad 240
.LCPI1_5:
  .quad 104
  .quad 160
  .quad 216
  .quad 272
energy:
  pushq %r14
  pushq %rbx
  vxorpd %xmm0, %xmm0, %xmm0
  movq $-224, %rax
  movl $1, %ecx
  xorl %edx, %edx
  vpbroadcastq .LCPI1_1(%rip), %ymm1
  movl $bodies, %esi
  vmovq %rsi, %xmm2
  vpbroadcastq %xmm2, %ymm11
  vmovdqu .LCPI1_2(%rip), %ymm6
  vmovdqu .LCPI1_3(%rip), %ymm2
  vmovdqu .LCPI1_4(%rip), %ymm12
  jmp .LBB1_1
.LBB1_9:
  leaq 1(%rdx), %rsi
  addq $56, %rax
  incq %rcx
  cmpq $4, %rdx
  movq %rsi, %rdx
  je .LBB1_10
.LBB1_1:
  vmovapd %xmm0, %xmm7
  imulq $56, %rdx, %rsi
  vmovsd bodies+24(%rsi), %xmm8
  vmovsd bodies+32(%rsi), %xmm0
  vmovupd bodies+40(%rsi), %xmm3
  vmovhpd .LCPI1_0(%rip), %xmm3, %xmm10
  vmulpd %xmm3, %xmm10, %xmm10
  vfmadd213sd %xmm10, %xmm8, %xmm8
  vfmadd231sd %xmm0, %xmm0, %xmm8
  vshufpd $1, %xmm10, %xmm10, %xmm0
  vfmadd213sd %xmm7, %xmm8, %xmm0
  cmpq $3, %rdx
  ja .LBB1_9
  vmovsd bodies(%rsi), %xmm7
  vmovupd bodies+8(%rsi), %xmm8
  movl $4, %edi
  subq %rdx, %rdi
  movq %rdi, %rsi
  andq $-4, %rsi
  je .LBB1_3
  leaq -1(%rsi), %r8
  vmovupd %xmm7, -88(%rsp)
  vbroadcastsd %xmm7, %ymm10
  vmovdqa %ymm11, %ymm7
  vbroadcastsd %xmm8, %ymm11
  vmovupd %ymm8, -40(%rsp)
  vpermpd $85, %ymm8, %ymm13
  vmovupd %ymm3, -72(%rsp)
  vpermpd $85, %ymm3, %ymm14
  vmovdqa %ymm12, %ymm8
  vpxor %xmm12, %xmm12, %xmm12
  xorl %r9d, %r9d
.LBB1_7:
  leaq (%rdx,%r9), %r10
  vmovq %r10, %xmm15
  vpbroadcastq %xmm15, %ymm15
  vpmuludq %ymm1, %ymm15, %ymm3
  vpsrlq $32, %ymm15, %ymm15
  vpaddq %ymm7, %ymm3, %ymm3
  vpmuludq %ymm1, %ymm15, %ymm15
  vpsllq $32, %ymm15, %ymm15
  vpaddq %ymm3, %ymm15, %ymm15
  vpaddq %ymm6, %ymm15, %ymm3
  vmovq %xmm3, %r10
  vextracti128 $1, %ymm3, %xmm4
  vpextrq $1, %xmm3, %r11
  vmovq %xmm4, %rbx
  vmovsd (%r10), %xmm3
  vpaddq %ymm2, %ymm15, %ymm5
  vmovdqa %ymm6, %ymm9
  vmovsd (%rbx), %xmm6
  vpextrq $1, %xmm5, %r10
  vpextrq $1, %xmm4, %rbx
  vmovq %xmm5, %r14
  vextracti128 $1, %ymm5, %xmm4
  vmovhpd (%r11), %xmm3, %xmm3
  vpextrq $1, %xmm4, %r11
  vmovhpd (%rbx), %xmm6, %xmm5
  vmovq %xmm4, %rbx
  vmovsd (%rbx), %xmm4
  vmovsd (%r14), %xmm6
  vinsertf128 $1, %xmm5, %ymm3, %ymm3
  vpaddq %ymm8, %ymm15, %ymm5
  vmovq %xmm5, %rbx
  vmovhpd (%r10), %xmm6, %xmm6
  vpextrq $1, %xmm5, %r10
  vmovhpd (%r11), %xmm4, %xmm4
  vextracti128 $1, %ymm5, %xmm5
  vpextrq $1, %xmm5, %r11
  vinsertf128 $1, %xmm4, %ymm6, %ymm4
  vmovq %xmm5, %r14
  vmovsd (%r14), %xmm5
  vmovsd (%rbx), %xmm6
  vpaddq .LCPI1_5(%rip), %ymm15, %ymm15
  vpextrq $1, %xmm15, %rbx
  vmovhpd (%r10), %xmm6, %xmm6
  vmovq %xmm15, %r10
  vextracti128 $1, %ymm15, %xmm15
  vmovhpd (%r11), %xmm5, %xmm5
  vmovq %xmm15, %r11
  vpextrq $1, %xmm15, %r14
  vinsertf128 $1, %xmm5, %ymm6, %ymm5
  vmovsd (%r11), %xmm6
  vmovsd (%r10), %xmm15
  vmovhpd (%r14), %xmm6, %xmm6
  vmovhpd (%rbx), %xmm15, %xmm15
  vinsertf128 $1, %xmm6, %ymm15, %ymm6
  vsubpd %ymm3, %ymm10, %ymm3
  vsubpd %ymm4, %ymm11, %ymm4
  vmulpd %ymm3, %ymm3, %ymm3
  vfmadd231pd %ymm4, %ymm4, %ymm3
  vsubpd %ymm5, %ymm13, %ymm4
  vfmadd231pd %ymm4, %ymm4, %ymm3
  vsqrtpd %ymm3, %ymm3
  vmulpd %ymm6, %ymm14, %ymm4
  vmovdqa %ymm9, %ymm6
  vdivpd %ymm3, %ymm4, %ymm3
  vsubpd %ymm3, %ymm12, %ymm12
  addq $4, %r9
  cmpq %r8, %r9
  jle .LBB1_7
  vextractf128 $1, %ymm12, %xmm3
  vaddpd %xmm3, %xmm12, %xmm3
  vshufpd $1, %xmm3, %xmm3, %xmm4
  vaddsd %xmm4, %xmm3, %xmm3
  vaddsd %xmm3, %xmm0, %xmm0
  cmpq %rsi, %rdi
  vmovdqa %ymm7, %ymm11
  vmovdqa %ymm8, %ymm12
  vmovupd -88(%rsp), %xmm7
  vmovupd -40(%rsp), %ymm8
  vmovupd -72(%rsp), %ymm3
  je .LBB1_9
  jmp .LBB1_4
.LBB1_3:
  xorl %esi, %esi
.LBB1_4:
  vshufpd $1, %xmm3, %xmm3, %xmm9
  imulq $56, %rsi, %rdi
  addq %rax, %rdi
  addq %rcx, %rsi
  imulq $56, %rsi, %rsi
  xorl %r8d, %r8d
.LBB1_5:
  vsubpd bodies+8(%rsi,%r8), %xmm8, %xmm3
  vmulpd %xmm3, %xmm3, %xmm3
  vshufpd $1, %xmm3, %xmm3, %xmm4
  vsubsd bodies(%rsi,%r8), %xmm7, %xmm5
  vaddsd %xmm3, %xmm4, %xmm3
  vfmadd213sd %xmm3, %xmm5, %xmm5
  vsqrtsd %xmm5, %xmm5, %xmm3
  vmulsd bodies+48(%rsi,%r8), %xmm9, %xmm4
  vdivsd %xmm3, %xmm4, %xmm3
  vsubsd %xmm3, %xmm0, %xmm0
  addq $56, %r8
  movq %rdi, %r9
  addq %r8, %r9
  jne .LBB1_5
  jmp .LBB1_9
.LBB1_10:
  popq %rbx
  popq %r14
  vzeroupper
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

