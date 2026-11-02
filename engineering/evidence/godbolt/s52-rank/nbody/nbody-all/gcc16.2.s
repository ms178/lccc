energy:
  vmovsd bodies+32(%rip), %xmm1
  vmovsd bodies+24(%rip), %xmm0
  movl $bodies, %eax
  movl $4, %ecx
  vmovsd bodies+48(%rip), %xmm10
  vmovsd .LC0(%rip), %xmm9
  movl $1, %esi
  vmulsd %xmm1, %xmm1, %xmm1
  vmovsd bodies+40(%rip), %xmm3
  vmulsd %xmm9, %xmm10, %xmm2
  vfmadd132sd %xmm0, %xmm1, %xmm0
  vfmadd132sd %xmm3, %xmm0, %xmm3
  vxorpd %xmm0, %xmm0, %xmm0
  vfmadd132sd %xmm2, %xmm0, %xmm3
.L2:
  vmovsd (%rax), %xmm5
  vmovsd 8(%rax), %xmm6
  vmovsd 16(%rax), %xmm4
  cmpl $4, %esi
  je .L8
  vmovupd 56(%rax), %xmm14
  vmovupd 104(%rax), %xmm2
  vmovddup %xmm5, %xmm12
  movl %ecx, %edx
  vmovupd 120(%rax), %xmm0
  vmovddup %xmm6, %xmm8
  vmovddup %xmm4, %xmm7
  shrl %edx
  vmovsd %xmm14, %xmm2, %xmm1
  vsubpd %xmm1, %xmm12, %xmm13
  vmovddup %xmm10, %xmm11
  vmovhps 160(%rax), %xmm2, %xmm2
  vshufpd $1, %xmm0, %xmm14, %xmm1
  vsubpd %xmm1, %xmm8, %xmm1
  vmovlpd 72(%rax), %xmm0, %xmm0
  vsubpd %xmm0, %xmm7, %xmm0
  vmulpd %xmm11, %xmm2, %xmm2
  vmulpd %xmm1, %xmm1, %xmm1
  vfmadd231pd %xmm13, %xmm13, %xmm1
  vfmadd132pd %xmm0, %xmm1, %xmm0
  vsqrtpd %xmm0, %xmm1
  vdivpd %xmm1, %xmm2, %xmm1
  vsubsd %xmm1, %xmm3, %xmm3
  vunpckhpd %xmm1, %xmm1, %xmm1
  vsubsd %xmm1, %xmm3, %xmm3
  cmpl $2, %edx
  jne .L4
  vmovupd 168(%rax), %xmm13
  vmovupd 216(%rax), %xmm0
  vmovupd 232(%rax), %xmm2
  vmovsd %xmm13, %xmm0, %xmm1
  vsubpd %xmm1, %xmm12, %xmm1
  vmovhps 272(%rax), %xmm0, %xmm0
  vmulpd %xmm11, %xmm0, %xmm0
  vshufpd $1, %xmm2, %xmm13, %xmm13
  vsubpd %xmm13, %xmm8, %xmm8
  vmovlpd 184(%rax), %xmm2, %xmm2
  vsubpd %xmm2, %xmm7, %xmm7
  vmulpd %xmm8, %xmm8, %xmm8
  vfmadd132pd %xmm1, %xmm8, %xmm1
  vfmadd132pd %xmm7, %xmm1, %xmm7
  vsqrtpd %xmm7, %xmm7
  vdivpd %xmm7, %xmm0, %xmm0
  vsubsd %xmm0, %xmm3, %xmm3
  vunpckhpd %xmm0, %xmm0, %xmm0
  vsubsd %xmm0, %xmm3, %xmm3
.L4:
  addl %edx, %edx
  cmpl %ecx, %edx
  je .L5
  addl %esi, %edx
.L3:
  imulq $56, %rdx, %rdx
  vmovsd 80(%rax), %xmm2
  vmovsd 96(%rax), %xmm1
  vsubsd bodies+8(%rdx), %xmm6, %xmm6
  vsubsd bodies(%rdx), %xmm5, %xmm5
  vsubsd bodies+16(%rdx), %xmm4, %xmm0
  vmulsd bodies+48(%rdx), %xmm10, %xmm10
  vmovsd 88(%rax), %xmm4
  vmulsd %xmm6, %xmm6, %xmm6
  vmulsd %xmm4, %xmm4, %xmm4
  vfmadd132sd %xmm5, %xmm6, %xmm5
  vfmadd132sd %xmm2, %xmm4, %xmm2
  vfmadd132sd %xmm0, %xmm5, %xmm0
  vfmadd132sd %xmm1, %xmm2, %xmm1
  vsqrtsd %xmm0, %xmm0, %xmm0
  vdivsd %xmm0, %xmm10, %xmm0
  vmovsd 104(%rax), %xmm10
  vsubsd %xmm0, %xmm3, %xmm0
  vmulsd %xmm9, %xmm10, %xmm3
  vfmadd132sd %xmm1, %xmm0, %xmm3
  subl $1, %ecx
  je .L11
  addl $1, %esi
  addq $56, %rax
  jmp .L2
.L5:
  vmovsd 88(%rax), %xmm2
  vmovsd 80(%rax), %xmm1
  addl $1, %esi
  subl $1, %ecx
  vmovsd 104(%rax), %xmm10
  vmovsd 96(%rax), %xmm0
  addq $56, %rax
  vmulsd %xmm2, %xmm2, %xmm2
  vmulsd %xmm9, %xmm10, %xmm4
  vfmadd132sd %xmm1, %xmm2, %xmm1
  vfmadd132sd %xmm0, %xmm1, %xmm0
  vfmadd231sd %xmm4, %xmm0, %xmm3
  jmp .L2
.L11:
  vmovapd %xmm3, %xmm0
  ret
.L8:
  movl $4, %edx
  jmp .L3
.LC4:
main:
  subq $8, %rsp
  vmovupd bodies+40(%rip), %xmm0
  vmovupd bodies+24(%rip), %xmm1
  vmovq bodies+104(%rip), %xmm3
  vmovhps bodies+80(%rip), %xmm1, %xmm2
  vxorpd %xmm4, %xmm4, %xmm4
  movl $.LC4, %edi
  vshufpd $1, bodies+88(%rip), %xmm1, %xmm1
  vshufpd $1, %xmm3, %xmm0, %xmm3
  vmulpd %xmm3, %xmm2, %xmm2
  vmovhps bodies+96(%rip), %xmm0, %xmm0
  vmulpd %xmm3, %xmm0, %xmm0
  vmulpd %xmm3, %xmm1, %xmm1
  vaddsd %xmm4, %xmm2, %xmm5
  vunpckhpd %xmm2, %xmm2, %xmm2
  vaddsd %xmm4, %xmm1, %xmm7
  vunpckhpd %xmm1, %xmm1, %xmm1
  vaddsd %xmm2, %xmm5, %xmm5
  vaddsd %xmm4, %xmm0, %xmm2
  vunpckhpd %xmm0, %xmm0, %xmm0
  vmovq bodies+216(%rip), %xmm4
  vaddsd %xmm1, %xmm7, %xmm7
  vmovupd bodies+136(%rip), %xmm1
  vmovhps bodies+192(%rip), %xmm1, %xmm3
  vshufpd $1, bodies+200(%rip), %xmm1, %xmm1
  vaddsd %xmm0, %xmm2, %xmm2
  vmovupd bodies+152(%rip), %xmm0
  vshufpd $1, %xmm4, %xmm0, %xmm4
  vmulpd %xmm4, %xmm3, %xmm3
  vmovhps bodies+208(%rip), %xmm0, %xmm0
  vmulpd %xmm4, %xmm1, %xmm1
  vmulpd %xmm4, %xmm0, %xmm0
  vaddsd %xmm5, %xmm3, %xmm6
  vunpckhpd %xmm3, %xmm3, %xmm3
  vaddsd %xmm7, %xmm1, %xmm5
  vunpckhpd %xmm1, %xmm1, %xmm1
  vmovupd bodies+248(%rip), %xmm7
  vmovlpd bodies+272(%rip), %xmm7, %xmm4
  vmovhps bodies+272(%rip), %xmm7, %xmm7
  vaddsd %xmm2, %xmm0, %xmm2
  vunpckhpd %xmm0, %xmm0, %xmm0
  vaddsd %xmm6, %xmm3, %xmm3
  vaddsd %xmm5, %xmm1, %xmm1
  vaddsd %xmm2, %xmm0, %xmm0
  vmovsd bodies+272(%rip), %xmm2
  vfnmsub231sd bodies+264(%rip), %xmm2, %xmm0
  vdivsd .LC3(%rip), %xmm0, %xmm0
  vmovsd %xmm0, bodies+40(%rip)
  vunpcklpd %xmm1, %xmm3, %xmm1
  vmovddup .LC3(%rip), %xmm3
  vfnmsub231pd %xmm7, %xmm4, %xmm1
  vdivpd %xmm3, %xmm1, %xmm1
  vmovupd %xmm1, bodies+24(%rip)
  call energy
  movl $1, %eax
  call printf
  vmovapd bodies(%rip), %xmm8
  movq bodies+16(%rip), %rsi
  movl $5000000, %ecx
  vmovddup .LC6(%rip), %xmm7
  vmovsd .LC6(%rip), %xmm6
.L13:
  movl $bodies, %eax
  movl $1, %edx
.L19:
  vmovupd (%rax), %xmm3
  vsubpd 56(%rax), %xmm3, %xmm10
  leal 1(%rdx), %edi
  vmovsd 16(%rax), %xmm14
  vsubsd 72(%rax), %xmm14, %xmm0
  vmovsd 48(%rax), %xmm4
  vunpckhpd %xmm10, %xmm10, %xmm1
  vmovapd %xmm10, %xmm2
  vmulsd %xmm1, %xmm1, %xmm1
  vmovddup %xmm4, %xmm5
  vmulpd %xmm5, %xmm10, %xmm11
  vfmadd132sd %xmm10, %xmm1, %xmm2
  vfmadd231sd %xmm0, %xmm0, %xmm2
  vsqrtsd %xmm2, %xmm2, %xmm1
  vmulsd %xmm2, %xmm1, %xmm1
  vmovsd 104(%rax), %xmm2
  vmovddup %xmm2, %xmm9
  vmulpd %xmm10, %xmm9, %xmm9
  vmulsd %xmm2, %xmm0, %xmm2
  vdivsd %xmm1, %xmm6, %xmm1
  vmulsd %xmm4, %xmm0, %xmm0
  vfnmadd213sd 40(%rax), %xmm1, %xmm2
  vfmadd213sd 96(%rax), %xmm1, %xmm0
  vmovddup %xmm1, %xmm12
  vfnmadd213pd 24(%rax), %xmm12, %xmm9
  vfmadd213pd 80(%rax), %xmm12, %xmm11
  vmovsd %xmm2, 40(%rax)
  vmovsd %xmm0, 96(%rax)
  vmovupd %xmm9, 24(%rax)
  vmovupd %xmm11, 80(%rax)
  cmpl $4, %edx
  je .L16
  vsubpd 112(%rax), %xmm3, %xmm12
  vsubsd 128(%rax), %xmm14, %xmm0
  vmulpd %xmm5, %xmm12, %xmm11
  vunpckhpd %xmm12, %xmm12, %xmm1
  vmovapd %xmm12, %xmm10
  vmulsd %xmm1, %xmm1, %xmm1
  vfmadd132sd %xmm12, %xmm1, %xmm10
  vfmadd231sd %xmm0, %xmm0, %xmm10
  vsqrtsd %xmm10, %xmm10, %xmm1
  vmulsd %xmm10, %xmm1, %xmm1
  vmovsd 160(%rax), %xmm10
  vmovddup %xmm10, %xmm15
  vmulpd %xmm12, %xmm15, %xmm15
  vmulsd %xmm0, %xmm10, %xmm10
  vdivsd %xmm1, %xmm6, %xmm1
  vmulsd %xmm0, %xmm4, %xmm0
  vfmadd213sd 152(%rax), %xmm1, %xmm0
  vmovddup %xmm1, %xmm13
  vfnmadd231sd %xmm1, %xmm10, %xmm2
  vfnmadd231pd %xmm13, %xmm15, %xmm9
  vmovsd %xmm0, 152(%rax)
  vfmadd213pd 136(%rax), %xmm13, %xmm11
  vmovsd %xmm2, 40(%rax)
  vmovupd %xmm9, 24(%rax)
  vmovupd %xmm11, 136(%rax)
  cmpl $3, %edx
  je .L18
  vsubpd 168(%rax), %xmm3, %xmm11
  vsubsd 184(%rax), %xmm14, %xmm0
  vmovsd 216(%rax), %xmm13
  vunpckhpd %xmm11, %xmm11, %xmm12
  vmovapd %xmm11, %xmm1
  vmovddup %xmm13, %xmm15
  vmulsd %xmm12, %xmm12, %xmm12
  vmulsd %xmm0, %xmm13, %xmm13
  vmulpd %xmm11, %xmm15, %xmm15
  vmulpd %xmm11, %xmm5, %xmm10
  vfmadd132sd %xmm11, %xmm12, %xmm1
  vfmadd231sd %xmm0, %xmm0, %xmm1
  vmulsd %xmm0, %xmm4, %xmm0
  vsqrtsd %xmm1, %xmm1, %xmm12
  vmulsd %xmm1, %xmm12, %xmm1
  vdivsd %xmm1, %xmm6, %xmm1
  vfmadd213sd 208(%rax), %xmm1, %xmm0
  vmovddup %xmm1, %xmm12
  vfnmadd231sd %xmm1, %xmm13, %xmm2
  vfnmadd231pd %xmm12, %xmm15, %xmm9
  vmovsd %xmm0, 208(%rax)
  vfmadd213pd 192(%rax), %xmm10, %xmm12
  vmovsd %xmm2, 40(%rax)
  vmovupd %xmm9, 24(%rax)
  vmovupd %xmm12, 192(%rax)
  cmpl $2, %edx
  je .L18
  vsubpd 224(%rax), %xmm3, %xmm3
  vsubsd 240(%rax), %xmm14, %xmm14
  vmulpd %xmm3, %xmm5, %xmm5
  vunpckhpd %xmm3, %xmm3, %xmm0
  vmovapd %xmm3, %xmm1
  vmulsd %xmm0, %xmm0, %xmm0
  vfmadd132sd %xmm3, %xmm0, %xmm1
  vfmadd231sd %xmm14, %xmm14, %xmm1
  vsqrtsd %xmm1, %xmm1, %xmm0
  vmulsd %xmm1, %xmm0, %xmm0
  vmovsd 272(%rax), %xmm1
  vmovddup %xmm1, %xmm11
  vmulpd %xmm3, %xmm11, %xmm3
  vmulsd %xmm1, %xmm14, %xmm1
  vdivsd %xmm0, %xmm6, %xmm0
  vmulsd %xmm4, %xmm14, %xmm14
  vmovddup %xmm0, %xmm10
  vfnmadd132sd %xmm0, %xmm2, %xmm1
  vfmadd213sd 264(%rax), %xmm14, %xmm0
  vfnmadd132pd %xmm10, %xmm9, %xmm3
  vfmadd213pd 248(%rax), %xmm5, %xmm10
  vmovsd %xmm1, 40(%rax)
  vmovupd %xmm3, 24(%rax)
  vmovupd %xmm10, 248(%rax)
  vmovsd %xmm0, 264(%rax)
.L18:
  addq $56, %rax
  movl %edi, %edx
  jmp .L19
.L16:
  vmovapd bodies+80(%rip), %xmm0
  vmovq %rsi, %xmm5
  vfmadd213pd bodies+56(%rip), %xmm7, %xmm0
  vmovapd bodies+112(%rip), %xmm1
  vfmadd231sd bodies+40(%rip), %xmm6, %xmm5
  vfmadd231pd bodies+24(%rip), %xmm7, %xmm8
  vmovupd %xmm0, bodies+56(%rip)
  vmovsd bodies+96(%rip), %xmm0
  vfmadd213sd bodies+72(%rip), %xmm6, %xmm0
  vmovq %xmm5, %rsi
  vmovsd %xmm5, bodies+16(%rip)
  vmovapd %xmm8, bodies(%rip)
  vmovsd %xmm0, bodies+72(%rip)
  vmovupd bodies+136(%rip), %xmm0
  vfmadd132pd %xmm7, %xmm1, %xmm0
  vmovapd bodies+224(%rip), %xmm1
  vmovapd %xmm0, bodies+112(%rip)
  vmovsd bodies+152(%rip), %xmm0
  vfmadd213sd bodies+128(%rip), %xmm6, %xmm0
  vmovsd %xmm0, bodies+128(%rip)
  vmovapd bodies+192(%rip), %xmm0
  vfmadd213pd bodies+168(%rip), %xmm7, %xmm0
  vmovupd %xmm0, bodies+168(%rip)
  vmovsd bodies+208(%rip), %xmm0
  vfmadd213sd bodies+184(%rip), %xmm6, %xmm0
  vmovsd %xmm0, bodies+184(%rip)
  vmovupd bodies+248(%rip), %xmm0
  vfmadd132pd %xmm7, %xmm1, %xmm0
  vmovapd %xmm0, bodies+224(%rip)
  vmovsd bodies+264(%rip), %xmm0
  vfmadd213sd bodies+240(%rip), %xmm6, %xmm0
  vmovsd %xmm0, bodies+240(%rip)
  subl $1, %ecx
  jne .L13
  call energy
  movl $.LC4, %edi
  movl $1, %eax
  call printf
  xorl %eax, %eax
  addq $8, %rsp
  ret
bodies:
.LC0:
.LC3:
.LC6:
