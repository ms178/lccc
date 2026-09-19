rint:
  retq

nearbyint:
  retq

roundeven:
  retq

floor:
  retq

ceil:
  retq

trunc:
  retq

copysign:
  vmovq %xmm0, %rax
  movb $63, %cl
  bzhiq %rcx, %rax, %rax
  vmovmskpd %xmm1, %ecx
  shlq $63, %rcx
  orq %rax, %rcx
  vmovq %rcx, %xmm0
  retq

fma:
  vfmadd213sd %xmm2, %xmm1, %xmm0
  retq

.LCPI8_0:
.LCPI8_3:
.LCPI8_1:
.LCPI8_2:
main:
  pushq %rbx
  subq $16, %rsp
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  xorl %eax, %eax
  vmovdqu .LCPI8_0(%rip), %xmm0
  vbroadcastsd .LCPI8_1(%rip), %ymm1
  vmovupd .LCPI8_2(%rip), %ymm2
.LBB8_1:
  vmovd %eax, %xmm3
  vpbroadcastd %xmm3, %xmm3
  vpaddd %xmm0, %xmm3, %xmm3
  vcvtdq2pd %xmm3, %ymm3
  vfmadd213pd %ymm2, %ymm1, %ymm3
  vmovupd %ymm3, buf(,%rax,8)
  leal 4(%rax), %ecx
  vmovd %ecx, %xmm3
  vpbroadcastd %xmm3, %xmm3
  vpaddd %xmm0, %xmm3, %xmm3
  vcvtdq2pd %xmm3, %ymm3
  vfmadd213pd %ymm2, %ymm1, %ymm3
  vmovupd %ymm3, buf+32(,%rax,8)
  leal 8(%rax), %ecx
  vmovd %ecx, %xmm3
  vpbroadcastd %xmm3, %xmm3
  vpaddd %xmm0, %xmm3, %xmm3
  vcvtdq2pd %xmm3, %ymm3
  vfmadd213pd %ymm2, %ymm1, %ymm3
  vmovupd %ymm3, buf+64(,%rax,8)
  leaq 12(%rax), %rcx
  vmovd %ecx, %xmm3
  vpbroadcastd %xmm3, %xmm3
  vpaddd %xmm0, %xmm3, %xmm3
  vcvtdq2pd %xmm3, %ymm3
  vfmadd213pd %ymm2, %ymm1, %ymm3
  vmovupd %ymm3, buf+96(,%rax,8)
  addq $16, %rax
  cmpq $4092, %rcx
  jb .LBB8_1
  vxorpd %xmm1, %xmm1, %xmm1
  xorl %ebx, %ebx
.LBB8_3:
  vmovsd %xmm1, 8(%rsp)
  vzeroupper
  callq round_family_pass
  vmovsd 8(%rsp), %xmm1
  vaddsd %xmm1, %xmm0, %xmm1
  movl %ebx, %eax
  andl $4095, %eax
  vmovsd buf(,%rax,8), %xmm0
  vxorpd .LCPI8_3(%rip), %xmm0, %xmm0
  vmovlpd %xmm0, buf(,%rax,8)
  incl %ebx
  cmpl $20000, %ebx
  jne .LBB8_3
  movl $.L.str, %edi
  vmovapd %xmm1, %xmm0
  movb $1, %al
  callq printf
  xorl %eax, %eax
  addq $16, %rsp
  popq %rbx
  retq

.LCPI9_0:
.LCPI9_1:
round_family_pass:
  vxorpd %xmm2, %xmm2, %xmm2
  movq $-4, %rax
  vbroadcastsd .LCPI9_0(%rip), %ymm0
  vbroadcastsd .LCPI9_1(%rip), %ymm1
.LBB9_1:
  vmovupd buf+32(,%rax,8), %ymm3
  vroundpd $9, %ymm3, %ymm4
  vroundpd $10, %ymm3, %ymm5
  vroundpd $11, %ymm3, %ymm6
  vroundpd $4, %ymm3, %ymm7
  vaddpd %ymm2, %ymm7, %ymm2
  vroundpd $12, %ymm3, %ymm7
  vroundpd $8, %ymm3, %ymm8
  vaddpd %ymm7, %ymm8, %ymm7
  vaddpd %ymm7, %ymm2, %ymm2
  vandnpd %ymm3, %ymm0, %ymm3
  vandpd %ymm0, %ymm6, %ymm7
  vorpd %ymm3, %ymm7, %ymm3
  vaddpd %ymm5, %ymm3, %ymm3
  vmovupd buf+64(,%rax,8), %ymm5
  vroundpd $9, %ymm5, %ymm7
  vroundpd $10, %ymm5, %ymm8
  vroundpd $11, %ymm5, %ymm9
  vroundpd $4, %ymm5, %ymm10
  vroundpd $12, %ymm5, %ymm11
  vroundpd $8, %ymm5, %ymm12
  vaddpd %ymm10, %ymm12, %ymm10
  vaddpd %ymm3, %ymm10, %ymm3
  vandnpd %ymm5, %ymm0, %ymm5
  vandpd %ymm0, %ymm9, %ymm10
  vorpd %ymm5, %ymm10, %ymm5
  vaddpd %ymm5, %ymm8, %ymm5
  vsubpd %ymm6, %ymm11, %ymm6
  vaddpd %ymm6, %ymm2, %ymm2
  vfmadd231pd %ymm4, %ymm1, %ymm2
  vaddpd %ymm2, %ymm3, %ymm2
  vaddpd %ymm2, %ymm5, %ymm2
  vfmsub213pd %ymm9, %ymm1, %ymm7
  vaddpd %ymm7, %ymm2, %ymm2
  addq $8, %rax
  cmpq $4092, %rax
  jb .LBB9_1
  vextractf128 $1, %ymm2, %xmm0
  vaddpd %xmm0, %xmm2, %xmm0
  vshufpd $1, %xmm0, %xmm0, %xmm1
  vaddsd %xmm1, %xmm0, %xmm0
  vzeroupper
  retq

.L.str:

