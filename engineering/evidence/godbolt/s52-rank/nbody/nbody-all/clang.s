.LCPI0_0:
.LCPI0_1:
.LCPI0_4:
.LCPI0_5:
main:
  subq $728, %rsp
  vmovddup bodies+48(%rip), %xmm0
  vxorpd %xmm1, %xmm1, %xmm1
  vfmadd231sd bodies+40(%rip), %xmm0, %xmm1
  vmovddup bodies+104(%rip), %xmm2
  vfmadd231sd bodies+96(%rip), %xmm2, %xmm1
  vmovddup bodies+160(%rip), %xmm3
  vfmadd231sd bodies+152(%rip), %xmm3, %xmm1
  vmovddup bodies+216(%rip), %xmm4
  vfmadd231sd bodies+208(%rip), %xmm4, %xmm1
  vmovddup bodies+272(%rip), %xmm5
  vfmadd231sd bodies+264(%rip), %xmm5, %xmm1
  vxorpd %xmm6, %xmm6, %xmm6
  vfmadd231pd bodies+24(%rip), %xmm0, %xmm6
  vfmadd231pd bodies+80(%rip), %xmm2, %xmm6
  vfmadd231pd bodies+136(%rip), %xmm3, %xmm6
  vfmadd231pd bodies+192(%rip), %xmm4, %xmm6
  vfmadd231pd bodies+248(%rip), %xmm5, %xmm6
  vdivpd .LCPI0_0(%rip), %xmm6, %xmm0
  vmovupd %xmm0, bodies+24(%rip)
  vdivsd .LCPI0_1(%rip), %xmm1, %xmm0
  vmovsd %xmm0, bodies+40(%rip)
  callq energy
  leaq .L.str(%rip), %rdi
  movb $1, %al
  callq printf@PLT
  vmovddup bodies+160(%rip), %xmm1
  vmovddup .LCPI0_5(%rip), %xmm0
  vmovapd %xmm1, 544(%rsp)
  vxorpd %xmm0, %xmm1, %xmm5
  vmovsd bodies+272(%rip), %xmm1
  vxorpd %xmm0, %xmm1, %xmm13
  vmovupd bodies+72(%rip), %xmm2
  vmovapd bodies+96(%rip), %xmm3
  vmovddup bodies+104(%rip), %xmm1
  vmovapd %xmm1, 528(%rsp)
  vxorpd %xmm0, %xmm1, %xmm1
  vmovapd %xmm1, 512(%rsp)
  vmovapd bodies+192(%rip), %xmm4
  vmovupd bodies+216(%rip), %xmm8
  vxorpd %xmm0, %xmm8, %xmm9
  vmovupd bodies+168(%rip), %xmm0
  vmovupd bodies+232(%rip), %xmm1
  vblendpd $1, bodies+184(%rip), %xmm1, %xmm6
  vmovapd %xmm6, 112(%rsp)
  vmovapd %xmm8, 496(%rsp)
  vblendpd $1, %xmm0, %xmm8, %xmm6
  vmovupd %ymm6, 304(%rsp)
  vshufpd $1, %xmm1, %xmm0, %xmm1
  vmovddup bodies+208(%rip), %xmm0
  vmovhpd bodies+264(%rip), %xmm0, %xmm12
  vmovhpd bodies+128(%rip), %xmm2, %xmm14
  vmovhpd bodies+152(%rip), %xmm3, %xmm0
  vmovapd %xmm0, 96(%rsp)
  vmovhpd bodies+248(%rip), %xmm4, %xmm6
  vmovddup bodies+216(%rip), %xmm0
  vblendpd $1, %xmm13, %xmm0, %xmm0
  vmovapd %xmm0, 480(%rsp)
  vmovapd %xmm5, 384(%rsp)
  vblendpd $1, %xmm5, %xmm3, %xmm0
  vmovapd %xmm0, 464(%rsp)
  movl $5000000, %eax
  vmovddup bodies+48(%rip), %xmm0
  vmovaps %xmm0, 336(%rsp)
  vmovsd bodies(%rip), %xmm0
  vmovaps %xmm0, 192(%rsp)
  vmovsd bodies+8(%rip), %xmm15
  vmovsd bodies+16(%rip), %xmm2
  vmovsd bodies+56(%rip), %xmm5
  vmovsd bodies+64(%rip), %xmm4
  vmovsd bodies+24(%rip), %xmm0
  vmovsd %xmm0, 32(%rsp)
  vmovsd bodies+32(%rip), %xmm0
  vmovsd %xmm0, 40(%rsp)
  vmovsd bodies+40(%rip), %xmm0
  vmovsd %xmm0, 24(%rsp)
  vmovsd bodies+80(%rip), %xmm0
  vmovsd %xmm0, 16(%rsp)
  vmovsd bodies+88(%rip), %xmm0
  vmovsd %xmm0, 8(%rsp)
  vmovsd bodies+112(%rip), %xmm3
  vmovsd bodies+120(%rip), %xmm0
  vmovsd bodies+136(%rip), %xmm8
  vmovsd %xmm8, 48(%rsp)
  vmovsd bodies+144(%rip), %xmm8
  vmovsd %xmm8, 80(%rsp)
  vmovsd bodies+200(%rip), %xmm11
  vmovaps %xmm11, 272(%rsp)
  vmovsd bodies+256(%rip), %xmm11
  vmovapd %xmm11, 64(%rsp)
  vmovapd %xmm3, 208(%rsp)
  vmovapd %xmm5, 176(%rsp)
  vunpcklpd %xmm3, %xmm5, %xmm5
  vmovapd %xmm13, 368(%rsp)
  vmovddup %xmm13, %xmm3
  vmovapd %xmm3, 448(%rsp)
  vmovapd %xmm9, 352(%rsp)
  vmovddup %xmm9, %xmm3
  vmovapd %xmm3, 432(%rsp)
.LBB0_1:
  vmovapd %xmm5, 592(%rsp)
  vmovapd %xmm12, 608(%rsp)
  vmovapd %xmm6, 624(%rsp)
  vmovapd %xmm15, 656(%rsp)
  vmovapd %xmm2, 672(%rsp)
  vmovapd %xmm4, 688(%rsp)
  vmovapd %xmm0, 704(%rsp)
  vmovapd %xmm14, 128(%rsp)
  vmovddup %xmm15, %xmm7
  vmovapd %xmm7, 256(%rsp)
  vmovddup %xmm4, %xmm4
  vmovapd %xmm4, 144(%rsp)
  vmovddup %xmm0, %xmm0
  vmovapd %xmm0, 160(%rsp)
  vunpcklpd %xmm0, %xmm4, %xmm0
  vsubpd %xmm0, %xmm7, %xmm10
  vmovddup 192(%rsp), %xmm0
  vmovapd %xmm0, 240(%rsp)
  vsubpd %xmm5, %xmm0, %xmm0
  vmulpd %xmm10, %xmm10, %xmm5
  vfmadd231pd %xmm0, %xmm0, %xmm5
  vmovddup %xmm2, %xmm2
  vmovapd %xmm2, 224(%rsp)
  vsubpd %xmm14, %xmm2, %xmm3
  vfmadd231pd %xmm3, %xmm3, %xmm5
  vsqrtpd %xmm5, %xmm7
  vmulpd %xmm7, %xmm5, %xmm5
  vmovddup .LCPI0_4(%rip), %xmm15
  vdivpd %xmm5, %xmm15, %xmm5
  vmovapd 512(%rsp), %xmm8
  vmulsd %xmm0, %xmm8, %xmm7
  vfmadd213sd 32(%rsp), %xmm5, %xmm7
  vmulsd %xmm8, %xmm10, %xmm9
  vmovapd %xmm10, %xmm2
  vfmadd213sd 40(%rsp), %xmm5, %xmm9
  vmulsd %xmm3, %xmm8, %xmm14
  vfmadd213sd 24(%rsp), %xmm5, %xmm14
  vshufpd $1, %xmm0, %xmm0, %xmm8
  vmovapd 384(%rsp), %xmm4
  vmulsd %xmm4, %xmm8, %xmm10
  vshufpd $1, %xmm5, %xmm5, %xmm12
  vfmadd231sd %xmm10, %xmm12, %xmm7
  vshufpd $1, %xmm2, %xmm2, %xmm10
  vmulsd %xmm4, %xmm10, %xmm13
  vfmadd231sd %xmm13, %xmm12, %xmm9
  vshufpd $1, %xmm3, %xmm3, %xmm13
  vmulsd %xmm4, %xmm13, %xmm13
  vfmadd231sd %xmm13, %xmm12, %xmm14
  vmovapd 336(%rsp), %xmm4
  vmulsd %xmm4, %xmm8, %xmm8
  vfmadd213sd 48(%rsp), %xmm12, %xmm8
  vmovsd %xmm8, 296(%rsp)
  vmulsd %xmm4, %xmm10, %xmm8
  vfmadd213sd 80(%rsp), %xmm12, %xmm8
  vmovsd %xmm8, 288(%rsp)
  vmulsd %xmm0, %xmm4, %xmm10
  vfmadd213sd 16(%rsp), %xmm5, %xmm10
  vmulsd %xmm2, %xmm4, %xmm13
  vfmadd213sd 8(%rsp), %xmm5, %xmm13
  vmulpd %xmm3, %xmm4, %xmm0
  vfmadd213pd 96(%rsp), %xmm5, %xmm0
  vmovupd 304(%rsp), %ymm8
  vmovapd 176(%rsp), %xmm2
  vunpcklpd %xmm8, %xmm2, %xmm2
  vblendpd $1, 208(%rsp), %xmm8, %xmm3
  vmovupd %ymm8, 304(%rsp)
  vsubpd %xmm3, %xmm2, %xmm3
  vmovapd %xmm3, 80(%rsp)
  vmovapd %xmm1, %xmm5
  vmovapd 144(%rsp), %xmm1
  vunpcklpd %xmm5, %xmm1, %xmm1
  vblendpd $1, 160(%rsp), %xmm5, %xmm2
  vmovapd %xmm5, 400(%rsp)
  vsubpd %xmm2, %xmm1, %xmm1
  vmovapd %xmm1, 416(%rsp)
  vmulpd %xmm1, %xmm1, %xmm1
  vfmadd231pd %xmm3, %xmm3, %xmm1
  vmovapd 112(%rsp), %xmm12
  vmovapd 128(%rsp), %xmm2
  vhsubpd %xmm12, %xmm2, %xmm3
  vfmadd231pd %xmm3, %xmm3, %xmm1
  vmovapd %xmm3, 640(%rsp)
  vsqrtpd %xmm1, %xmm2
  vmulpd %xmm2, %xmm1, %xmm1
  vdivpd %xmm1, %xmm15, %xmm2
  vmovapd %xmm2, 48(%rsp)
  vmovddup %xmm3, %xmm1
  vmulpd 464(%rsp), %xmm1, %xmm1
  vmovddup %xmm2, %xmm2
  vfmadd213pd %xmm0, %xmm1, %xmm2
  vmovapd %xmm2, 96(%rsp)
  vmovapd 240(%rsp), %xmm0
  vsubpd %xmm8, %xmm0, %xmm3
  vmovapd 256(%rsp), %xmm0
  vsubpd %xmm5, %xmm0, %xmm11
  vmovapd 224(%rsp), %xmm0
  vsubpd %xmm12, %xmm0, %xmm5
  vmovapd %xmm12, %xmm6
  vmovapd %xmm12, 112(%rsp)
  vmulpd %xmm11, %xmm11, %xmm0
  vfmadd231pd %xmm3, %xmm3, %xmm0
  vfmadd231pd %xmm5, %xmm5, %xmm0
  vsqrtpd %xmm0, %xmm1
  vmulpd %xmm1, %xmm0, %xmm0
  vdivpd %xmm0, %xmm15, %xmm12
  vshufpd $1, %xmm11, %xmm11, %xmm0
  vmovapd %xmm11, 256(%rsp)
  vmulsd %xmm0, %xmm4, %xmm2
  vshufpd $1, %xmm12, %xmm12, %xmm1
  vfmadd213sd 64(%rsp), %xmm1, %xmm2
  vmovsd %xmm2, 64(%rsp)
  vmovapd 352(%rsp), %xmm8
  vmulsd %xmm3, %xmm8, %xmm2
  vmovapd %xmm3, 240(%rsp)
  vfmadd231sd %xmm2, %xmm12, %xmm7
  vmulsd %xmm8, %xmm11, %xmm2
  vfmadd231sd %xmm2, %xmm12, %xmm9
  vmovapd %xmm5, 224(%rsp)
  vmulsd %xmm5, %xmm8, %xmm2
  vfmadd231sd %xmm2, %xmm12, %xmm14
  vshufpd $1, %xmm3, %xmm3, %xmm2
  vmovapd 368(%rsp), %xmm3
  vmulsd %xmm3, %xmm2, %xmm2
  vfmadd231sd %xmm2, %xmm1, %xmm7
  vmovsd %xmm7, 32(%rsp)
  vmulsd %xmm3, %xmm0, %xmm0
  vfmadd231sd %xmm0, %xmm1, %xmm9
  vmovsd %xmm9, 40(%rsp)
  vshufpd $1, %xmm5, %xmm5, %xmm0
  vmulsd %xmm3, %xmm0, %xmm0
  vfmadd231sd %xmm0, %xmm1, %xmm14
  vmovsd %xmm14, 24(%rsp)
  vmovapd 592(%rsp), %xmm15
  vmovddup %xmm15, %xmm0
  vmovupd 304(%rsp), %ymm7
  vsubpd %xmm7, %xmm0, %xmm3
  vmovapd 400(%rsp), %xmm11
  vmovapd 144(%rsp), %xmm0
  vsubpd %xmm11, %xmm0, %xmm5
  vmovddup 128(%rsp), %xmm0
  vsubpd %xmm6, %xmm0, %xmm1
  vmovapd %xmm1, 144(%rsp)
  vmulpd %xmm5, %xmm5, %xmm0
  vfmadd231pd %xmm3, %xmm3, %xmm0
  vfmadd231pd %xmm1, %xmm1, %xmm0
  vsqrtpd %xmm0, %xmm1
  vmulpd %xmm1, %xmm0, %xmm0
  vmovddup .LCPI0_4(%rip), %xmm1
  vdivpd %xmm0, %xmm1, %xmm2
  vshufpd $1, %xmm5, %xmm5, %xmm9
  vmovapd %xmm5, 560(%rsp)
  vmovapd 528(%rsp), %xmm6
  vmulsd %xmm6, %xmm9, %xmm1
  vshufpd $1, %xmm2, %xmm2, %xmm4
  vmovsd 64(%rsp), %xmm14
  vfmadd231sd %xmm1, %xmm4, %xmm14
  vmovsd %xmm14, 64(%rsp)
  vmovapd 384(%rsp), %xmm0
  vmulsd 80(%rsp), %xmm0, %xmm1
  vmovapd 48(%rsp), %xmm14
  vfmadd231sd %xmm1, %xmm14, %xmm10
  vmulsd 416(%rsp), %xmm0, %xmm1
  vfmadd231sd %xmm1, %xmm14, %xmm13
  vmovapd %xmm3, 576(%rsp)
  vmulsd %xmm3, %xmm8, %xmm1
  vfmadd231sd %xmm1, %xmm2, %xmm10
  vmulsd %xmm5, %xmm8, %xmm1
  vfmadd231sd %xmm1, %xmm2, %xmm13
  vshufpd $1, %xmm3, %xmm3, %xmm1
  vmovapd 368(%rsp), %xmm8
  vmulsd %xmm1, %xmm8, %xmm1
  vfmadd231sd %xmm1, %xmm4, %xmm10
  vmovsd %xmm10, 16(%rsp)
  vmulsd %xmm8, %xmm9, %xmm0
  vfmadd231sd %xmm0, %xmm4, %xmm13
  vmovsd %xmm13, 8(%rsp)
  vshufpd $3, %xmm15, %xmm15, %xmm0
  vsubpd %xmm7, %xmm0, %xmm14
  vmovapd 160(%rsp), %xmm0
  vsubpd %xmm11, %xmm0, %xmm4
  vpermilpd $3, 128(%rsp), %xmm0
  vmovapd 112(%rsp), %xmm5
  vsubpd %xmm5, %xmm0, %xmm1
  vmovapd %xmm1, 160(%rsp)
  vmulpd %xmm4, %xmm4, %xmm0
  vfmadd231pd %xmm14, %xmm14, %xmm0
  vfmadd231pd %xmm1, %xmm1, %xmm0
  vsqrtpd %xmm0, %xmm1
  vmulpd %xmm1, %xmm0, %xmm0
  vmovddup .LCPI0_4(%rip), %xmm1
  vdivpd %xmm0, %xmm1, %xmm1
  vmovddup %xmm5, %xmm0
  vmovapd 128(%rsp), %xmm3
  vsubpd %xmm0, %xmm3, %xmm0
  vmulpd 432(%rsp), %xmm0, %xmm3
  vunpcklpd %xmm1, %xmm2, %xmm0
  vfmadd213pd 96(%rsp), %xmm3, %xmm0
  vshufpd $3, %xmm5, %xmm5, %xmm3
  vmovapd 128(%rsp), %xmm15
  vsubpd %xmm3, %xmm15, %xmm3
  vmulpd 448(%rsp), %xmm3, %xmm3
  vunpckhpd %xmm1, %xmm2, %xmm5
  vfmadd213pd %xmm0, %xmm3, %xmm5
  vmovapd %xmm5, 96(%rsp)
  vmovapd 336(%rsp), %xmm3
  vmulsd 256(%rsp), %xmm3, %xmm0
  vfmadd213sd 272(%rsp), %xmm12, %xmm0
  vmulpd 224(%rsp), %xmm3, %xmm5
  vfmadd213pd 608(%rsp), %xmm12, %xmm5
  vmulpd 240(%rsp), %xmm3, %xmm3
  vfmadd213pd 624(%rsp), %xmm12, %xmm3
  vmulsd 560(%rsp), %xmm6, %xmm9
  vfmadd231sd %xmm9, %xmm2, %xmm0
  vmulpd 144(%rsp), %xmm6, %xmm9
  vfmadd213pd %xmm5, %xmm2, %xmm9
  vmulpd 576(%rsp), %xmm6, %xmm5
  vfmadd231pd %xmm5, %xmm2, %xmm3
  vmulsd 80(%rsp), %xmm6, %xmm2
  vmovsd 296(%rsp), %xmm12
  vmovapd 48(%rsp), %xmm5
  vfmadd231sd %xmm2, %xmm5, %xmm12
  vmovapd 416(%rsp), %xmm13
  vmulsd %xmm6, %xmm13, %xmm2
  vmovsd 288(%rsp), %xmm6
  vfmadd231sd %xmm2, %xmm5, %xmm6
  vmovapd 352(%rsp), %xmm5
  vmulsd %xmm5, %xmm14, %xmm2
  vfmadd231sd %xmm2, %xmm1, %xmm12
  vmulsd %xmm5, %xmm4, %xmm2
  vmovapd 656(%rsp), %xmm15
  vfmadd231sd %xmm2, %xmm1, %xmm6
  vshufpd $1, %xmm14, %xmm14, %xmm2
  vmovapd %xmm8, %xmm11
  vmulsd %xmm2, %xmm8, %xmm2
  vshufpd $1, %xmm1, %xmm1, %xmm5
  vfmadd231sd %xmm2, %xmm5, %xmm12
  vmovapd %xmm6, %xmm10
  vshufpd $1, %xmm4, %xmm4, %xmm7
  vmulsd %xmm7, %xmm8, %xmm8
  vfmadd231sd %xmm8, %xmm5, %xmm10
  vmovapd 544(%rsp), %xmm8
  vmulsd %xmm7, %xmm8, %xmm7
  vmovsd 64(%rsp), %xmm2
  vfmadd231sd %xmm7, %xmm5, %xmm2
  vmulsd %xmm4, %xmm8, %xmm4
  vfmadd231sd %xmm4, %xmm1, %xmm0
  vmulpd %xmm14, %xmm8, %xmm4
  vmovapd 672(%rsp), %xmm6
  vfmadd231pd %xmm4, %xmm1, %xmm3
  vmulpd 160(%rsp), %xmm8, %xmm4
  vmovapd 128(%rsp), %xmm14
  vfmadd213pd %xmm9, %xmm1, %xmm4
  vshufpd $1, %xmm13, %xmm13, %xmm1
  vmovapd %xmm0, %xmm9
  vmulsd %xmm1, %xmm11, %xmm0
  vmovapd 48(%rsp), %xmm13
  vshufpd $1, %xmm13, %xmm13, %xmm5
  vfmadd231sd %xmm0, %xmm5, %xmm9
  vmovapd 496(%rsp), %xmm7
  vmulsd %xmm1, %xmm7, %xmm0
  vmovapd %xmm2, %xmm8
  vfmadd231sd %xmm0, %xmm5, %xmm8
  vpermilpd $1, 80(%rsp), %xmm0
  vmulsd %xmm0, %xmm11, %xmm1
  vmulsd %xmm0, %xmm7, %xmm0
  vunpcklpd %xmm0, %xmm1, %xmm0
  vshufpd $3, %xmm13, %xmm13, %xmm1
  vmovapd %xmm3, %xmm5
  vfmadd231pd %xmm0, %xmm1, %xmm5
  vpermilpd $3, 640(%rsp), %xmm0
  vmulpd 480(%rsp), %xmm0, %xmm3
  vmovapd 704(%rsp), %xmm0
  vfmadd213pd %xmm4, %xmm1, %xmm3
  vmovapd 688(%rsp), %xmm4
  vmovsd .LCPI0_4(%rip), %xmm1
  vmovapd 192(%rsp), %xmm7
  vfmadd231sd 32(%rsp), %xmm1, %xmm7
  vmovapd %xmm7, 192(%rsp)
  vfmadd231sd 40(%rsp), %xmm1, %xmm15
  vfmadd231sd 24(%rsp), %xmm1, %xmm6
  vmovapd %xmm6, %xmm2
  vmovapd 176(%rsp), %xmm11
  vfmadd231sd 16(%rsp), %xmm1, %xmm11
  vfmadd231sd 8(%rsp), %xmm1, %xmm4
  vmovsd %xmm12, 48(%rsp)
  vmovapd 208(%rsp), %xmm13
  vfmadd231sd %xmm1, %xmm12, %xmm13
  vmovsd %xmm10, 80(%rsp)
  vfmadd231sd %xmm1, %xmm10, %xmm0
  vmovddup .LCPI0_4(%rip), %xmm10
  vfmadd231pd 96(%rsp), %xmm10, %xmm14
  vmovapd %xmm5, %xmm6
  vmovupd 304(%rsp), %ymm1
  vfmadd231pd %xmm10, %xmm5, %xmm1
  vmovupd %ymm1, 304(%rsp)
  vmovapd %xmm9, 272(%rsp)
  vmovapd %xmm8, 64(%rsp)
  vunpcklpd %xmm8, %xmm9, %xmm1
  vfmadd213pd 400(%rsp), %xmm10, %xmm1
  vmovapd %xmm3, %xmm12
  vmovapd 112(%rsp), %xmm5
  vfmadd231pd %xmm10, %xmm3, %xmm5
  vmovapd %xmm5, 112(%rsp)
  vmovapd %xmm13, 208(%rsp)
  vmovapd %xmm11, 176(%rsp)
  vunpcklpd %xmm13, %xmm11, %xmm5
  decl %eax
  jne .LBB0_1
  vmovaps 192(%rsp), %xmm3
  vmovsd %xmm3, bodies(%rip)
  vmovsd %xmm15, bodies+8(%rip)
  vmovsd %xmm2, bodies+16(%rip)
  vmovaps 176(%rsp), %xmm2
  vmovsd %xmm2, bodies+56(%rip)
  vmovsd %xmm4, bodies+64(%rip)
  vmovlpd %xmm14, bodies+72(%rip)
  vmovsd 32(%rsp), %xmm3
  vmovsd %xmm3, bodies+24(%rip)
  vmovsd 40(%rsp), %xmm3
  vmovsd %xmm3, bodies+32(%rip)
  vmovsd 24(%rsp), %xmm3
  vmovsd %xmm3, bodies+40(%rip)
  vmovsd 16(%rsp), %xmm3
  vmovsd %xmm3, bodies+80(%rip)
  vmovsd 8(%rsp), %xmm2
  vmovsd %xmm2, bodies+88(%rip)
  vmovaps 96(%rsp), %xmm2
  vmovlps %xmm2, bodies+96(%rip)
  vmovaps 208(%rsp), %xmm3
  vmovsd %xmm3, bodies+112(%rip)
  vmovsd %xmm0, bodies+120(%rip)
  vmovhpd %xmm14, bodies+128(%rip)
  vmovsd 48(%rsp), %xmm0
  vmovsd %xmm0, bodies+136(%rip)
  vmovsd 80(%rsp), %xmm0
  vmovsd %xmm0, bodies+144(%rip)
  vmovhps %xmm2, bodies+152(%rip)
  vinsertf128 $1, %xmm6, %ymm1, %ymm0
  vmovupd 304(%rsp), %ymm1
  vinsertf128 $1, 112(%rsp), %ymm1, %ymm1
  vunpcklpd %ymm0, %ymm1, %ymm2
  vmovupd %ymm2, bodies+168(%rip)
  vmovaps 272(%rsp), %xmm2
  vmovsd %xmm2, bodies+200(%rip)
  vunpckhpd %ymm0, %ymm1, %ymm0
  vmovlpd %xmm12, bodies+208(%rip)
  vmovupd %ymm0, bodies+224(%rip)
  vmovaps 64(%rsp), %xmm0
  vmovsd %xmm0, bodies+256(%rip)
  vmovhpd %xmm12, bodies+264(%rip)
  vzeroupper
  callq energy
  leaq .L.str(%rip), %rdi
  movb $1, %al
  callq printf@PLT
  xorl %eax, %eax
  addq $728, %rsp
  retq

.LCPI1_0:
energy:
  vmovddup bodies+160(%rip), %xmm3
  vmovsd bodies+96(%rip), %xmm0
  vmovsd bodies+80(%rip), %xmm1
  vmovsd bodies+88(%rip), %xmm2
  vmulsd %xmm2, %xmm2, %xmm2
  vfmadd231sd %xmm1, %xmm1, %xmm2
  vfmadd231sd %xmm0, %xmm0, %xmm2
  vmovsd %xmm2, -112(%rsp)
  vmovsd bodies+40(%rip), %xmm1
  vmovsd bodies+24(%rip), %xmm2
  vmovsd bodies+32(%rip), %xmm0
  vmulsd %xmm0, %xmm0, %xmm0
  vfmadd231sd %xmm2, %xmm2, %xmm0
  vfmadd231sd %xmm1, %xmm1, %xmm0
  vmovsd bodies+272(%rip), %xmm4
  vmovapd %xmm4, -72(%rsp)
  vmovsd bodies+240(%rip), %xmm7
  vmovapd %xmm7, -104(%rsp)
  vmovsd bodies+128(%rip), %xmm8
  vmovsd bodies+216(%rip), %xmm2
  vmovapd %xmm2, -24(%rsp)
  vmovsd bodies+184(%rip), %xmm1
  vmovsd bodies+104(%rip), %xmm5
  vmovsd bodies+48(%rip), %xmm10
  vmulsd .LCPI1_0(%rip), %xmm10, %xmm6
  vxorpd %xmm13, %xmm13, %xmm13
  vfmadd231sd %xmm6, %xmm0, %xmm13
  vmulsd %xmm4, %xmm2, %xmm0
  vmovapd %xmm3, -40(%rsp)
  vmulsd %xmm3, %xmm5, %xmm6
  vunpcklpd %xmm0, %xmm6, %xmm0
  vmovapd %xmm0, -88(%rsp)
  vmulsd %xmm3, %xmm10, %xmm0
  vmulsd %xmm5, %xmm10, %xmm6
  vunpcklpd %xmm0, %xmm6, %xmm12
  vsubsd %xmm7, %xmm1, %xmm0
  vmovddup bodies+72(%rip), %xmm2
  vmovapd %xmm2, -56(%rsp)
  vsubsd %xmm8, %xmm2, %xmm1
  vunpcklpd %xmm0, %xmm1, %xmm15
  vmovddup bodies+16(%rip), %xmm14
  vsubsd %xmm8, %xmm14, %xmm0
  vsubsd %xmm2, %xmm14, %xmm1
  vunpcklpd %xmm0, %xmm1, %xmm6
  vmovsd bodies+168(%rip), %xmm0
  vsubsd bodies+224(%rip), %xmm0, %xmm0
  vmovsd bodies+112(%rip), %xmm11
  vmovddup bodies+56(%rip), %xmm8
  vsubsd %xmm11, %xmm8, %xmm1
  vunpcklpd %xmm0, %xmm1, %xmm4
  vmovddup bodies(%rip), %xmm2
  vsubsd %xmm11, %xmm2, %xmm0
  vsubsd %xmm8, %xmm2, %xmm11
  vunpcklpd %xmm0, %xmm11, %xmm3
  vmovsd bodies+176(%rip), %xmm0
  vsubsd bodies+232(%rip), %xmm0, %xmm1
  vmovsd bodies+120(%rip), %xmm0
  vmovddup bodies+64(%rip), %xmm11
  vsubsd %xmm0, %xmm11, %xmm7
  vunpcklpd %xmm1, %xmm7, %xmm7
  vmovddup bodies+8(%rip), %xmm1
  vsubsd %xmm0, %xmm1, %xmm0
  vsubsd %xmm11, %xmm1, %xmm9
  vunpcklpd %xmm0, %xmm9, %xmm0
  vinsertf128 $1, %xmm7, %ymm0, %ymm0
  vinsertf128 $1, %xmm4, %ymm3, %ymm3
  vmulpd %ymm0, %ymm0, %ymm0
  vfmadd213pd %ymm0, %ymm3, %ymm3
  vinsertf128 $1, %xmm15, %ymm6, %ymm0
  vfmadd213pd %ymm3, %ymm0, %ymm0
  vsqrtpd %ymm0, %ymm0
  vinsertf128 $1, -88(%rsp), %ymm12, %ymm3
  vdivpd %ymm0, %ymm3, %ymm15
  vmovupd bodies+184(%rip), %xmm0
  vunpcklpd -104(%rsp), %xmm0, %xmm7
  vmovupd bodies+168(%rip), %xmm0
  vmovapd bodies+224(%rip), %xmm3
  vunpcklpd %xmm3, %xmm0, %xmm12
  vunpckhpd %xmm3, %xmm0, %xmm9
  vsubsd %xmm15, %xmm13, %xmm0
  vshufpd $1, %xmm15, %xmm15, %xmm3
  vsubsd %xmm3, %xmm0, %xmm0
  vmovupd bodies+216(%rip), %xmm3
  vunpcklpd %xmm10, %xmm3, %xmm4
  vmovapd -72(%rsp), %xmm13
  vunpcklpd %xmm13, %xmm10, %xmm6
  vmulpd %xmm6, %xmm4, %xmm4
  vsubpd %xmm12, %xmm2, %xmm2
  vsubpd %xmm9, %xmm1, %xmm1
  vmulpd %xmm1, %xmm1, %xmm1
  vfmadd231pd %xmm2, %xmm2, %xmm1
  vsubpd %xmm7, %xmm14, %xmm2
  vfmadd231pd %xmm2, %xmm2, %xmm1
  vsqrtpd %xmm1, %xmm1
  vdivpd %xmm1, %xmm4, %xmm1
  vsubsd %xmm1, %xmm0, %xmm0
  vshufpd $1, %xmm1, %xmm1, %xmm1
  vsubsd %xmm1, %xmm0, %xmm1
  vmovsd .LCPI1_0(%rip), %xmm6
  vmulsd %xmm6, %xmm5, %xmm0
  vfmadd231sd -112(%rsp), %xmm0, %xmm1
  vmovsd bodies+136(%rip), %xmm0
  vmovsd bodies+144(%rip), %xmm2
  vmulsd %xmm2, %xmm2, %xmm2
  vfmadd231sd %xmm0, %xmm0, %xmm2
  vmovsd bodies+152(%rip), %xmm0
  vfmadd231sd %xmm0, %xmm0, %xmm2
  vunpcklpd %xmm5, %xmm3, %xmm0
  vunpcklpd %xmm13, %xmm5, %xmm3
  vmulpd %xmm3, %xmm0, %xmm0
  vsubpd %xmm12, %xmm8, %xmm3
  vsubpd %xmm9, %xmm11, %xmm4
  vmulpd %xmm4, %xmm4, %xmm4
  vfmadd231pd %xmm3, %xmm3, %xmm4
  vmovapd -56(%rsp), %xmm3
  vsubpd %xmm7, %xmm3, %xmm3
  vfmadd231pd %xmm3, %xmm3, %xmm4
  vsqrtpd %xmm4, %xmm3
  vdivpd %xmm3, %xmm0, %xmm3
  vextractf128 $1, %ymm15, %xmm0
  vsubsd %xmm0, %xmm1, %xmm1
  vsubsd %xmm3, %xmm1, %xmm1
  vshufpd $1, %xmm3, %xmm3, %xmm3
  vsubsd %xmm3, %xmm1, %xmm1
  vmovapd -40(%rsp), %xmm11
  vmulsd %xmm6, %xmm11, %xmm3
  vfmadd231sd %xmm2, %xmm3, %xmm1
  vmovsd bodies+192(%rip), %xmm2
  vmovsd bodies+200(%rip), %xmm3
  vmulsd %xmm3, %xmm3, %xmm3
  vfmadd231sd %xmm2, %xmm2, %xmm3
  vmovsd bodies+208(%rip), %xmm2
  vfmadd231sd %xmm2, %xmm2, %xmm3
  vmovddup bodies+128(%rip), %xmm2
  vsubpd %xmm7, %xmm2, %xmm2
  vmovddup bodies+112(%rip), %xmm4
  vsubpd %xmm12, %xmm4, %xmm4
  vmovddup bodies+120(%rip), %xmm5
  vsubpd %xmm9, %xmm5, %xmm5
  vmulpd %xmm5, %xmm5, %xmm5
  vfmadd231pd %xmm4, %xmm4, %xmm5
  vfmadd231pd %xmm2, %xmm2, %xmm5
  vmovapd -24(%rsp), %xmm8
  vunpcklpd %xmm13, %xmm8, %xmm2
  vmulpd %xmm2, %xmm11, %xmm2
  vsqrtpd %xmm5, %xmm4
  vdivpd %xmm4, %xmm2, %xmm2
  vsubsd %xmm2, %xmm1, %xmm1
  vshufpd $1, %xmm2, %xmm2, %xmm2
  vsubsd %xmm2, %xmm1, %xmm1
  vmulsd %xmm6, %xmm8, %xmm2
  vfmadd231sd %xmm2, %xmm3, %xmm1
  vmovsd bodies+248(%rip), %xmm2
  vmovsd bodies+256(%rip), %xmm3
  vmulsd %xmm3, %xmm3, %xmm3
  vfmadd231sd %xmm2, %xmm2, %xmm3
  vmovsd bodies+264(%rip), %xmm2
  vfmadd231sd %xmm2, %xmm2, %xmm3
  vmulsd %xmm6, %xmm13, %xmm2
  vshufpd $1, %xmm0, %xmm0, %xmm0
  vsubsd %xmm0, %xmm1, %xmm0
  vfmadd231sd %xmm2, %xmm3, %xmm0
  vzeroupper
  retq

bodies:

.L.str:

