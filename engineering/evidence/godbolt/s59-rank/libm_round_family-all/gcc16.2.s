round_family_pass:
  movl $buf, %eax
  vmovsd .LC2(%rip), %xmm7
  vxorpd %xmm5, %xmm5, %xmm5
  movl $buf+32768, %edx
  vmovq .LC1(%rip), %xmm6
.L2:
  vmovsd (%rax), %xmm1
  addq $8, %rax
  vroundsd $9, %xmm1, %xmm1, %xmm3
  vroundsd $10, %xmm1, %xmm1, %xmm10
  vfmadd132sd %xmm7, %xmm10, %xmm3
  vroundsd $4, %xmm1, %xmm1, %xmm0
  vroundsd $12, %xmm1, %xmm1, %xmm9
  vroundsd $8, %xmm1, %xmm1, %xmm8
  vroundsd $11, %xmm1, %xmm1, %xmm4
  vandpd %xmm1, %xmm6, %xmm1
  vandnpd %xmm4, %xmm6, %xmm2
  vorpd %xmm1, %xmm2, %xmm1
  vaddsd %xmm3, %xmm0, %xmm0
  vaddsd %xmm9, %xmm0, %xmm0
  vaddsd %xmm8, %xmm0, %xmm0
  vaddsd %xmm1, %xmm0, %xmm0
  vsubsd %xmm4, %xmm0, %xmm0
  vaddsd %xmm0, %xmm5, %xmm5
  cmpq %rax, %rdx
  jne .L2
  vmovapd %xmm5, %xmm0
  ret
rint:
  vxorpd %xmm1, %xmm1, %xmm1
  vcomisd %xmm1, %xmm0
  vmovsd .LC3(%rip), %xmm1
  jb .L10
  vaddsd %xmm1, %xmm0, %xmm0
  vsubsd %xmm1, %xmm0, %xmm0
  ret
.L10:
  vsubsd %xmm1, %xmm0, %xmm0
  vaddsd %xmm1, %xmm0, %xmm0
  ret
nearbyint:
  vxorpd %xmm1, %xmm1, %xmm1
  vcomisd %xmm1, %xmm0
  vmovsd .LC3(%rip), %xmm1
  jb .L16
  vaddsd %xmm1, %xmm0, %xmm0
  vsubsd %xmm1, %xmm0, %xmm0
  ret
.L16:
  vsubsd %xmm1, %xmm0, %xmm0
  vaddsd %xmm1, %xmm0, %xmm0
  ret
roundeven:
  vxorpd %xmm1, %xmm1, %xmm1
  vcomisd %xmm1, %xmm0
  vmovsd .LC3(%rip), %xmm1
  jb .L22
  vaddsd %xmm1, %xmm0, %xmm0
  vsubsd %xmm1, %xmm0, %xmm0
  ret
.L22:
  vsubsd %xmm1, %xmm0, %xmm0
  vaddsd %xmm1, %xmm0, %xmm0
  ret
floor:
  vmovapd %xmm0, %xmm1
  vxorpd %xmm0, %xmm0, %xmm0
  vmovsd .LC3(%rip), %xmm2
  vcomisd %xmm0, %xmm1
  jb .L31
  vaddsd %xmm2, %xmm1, %xmm0
  vsubsd %xmm2, %xmm0, %xmm0
.L26:
  vcomisd %xmm1, %xmm0
  jbe .L23
  vsubsd .LC4(%rip), %xmm0, %xmm0
.L23:
  ret
.L31:
  vsubsd %xmm2, %xmm1, %xmm0
  vaddsd %xmm2, %xmm0, %xmm0
  jmp .L26
ceil:
  vmovapd %xmm0, %xmm1
  vxorpd %xmm0, %xmm0, %xmm0
  vmovsd .LC3(%rip), %xmm2
  vcomisd %xmm0, %xmm1
  jb .L40
  vaddsd %xmm2, %xmm1, %xmm0
  vsubsd %xmm2, %xmm0, %xmm0
.L35:
  vcomisd %xmm0, %xmm1
  jbe .L32
  vaddsd .LC4(%rip), %xmm0, %xmm0
.L32:
  ret
.L40:
  vsubsd %xmm2, %xmm1, %xmm0
  vaddsd %xmm2, %xmm0, %xmm0
  jmp .L35
trunc:
  vmovapd %xmm0, %xmm1
  vxorpd %xmm0, %xmm0, %xmm0
  vmovsd .LC3(%rip), %xmm2
  vcomisd %xmm0, %xmm1
  jb .L48
  vaddsd %xmm2, %xmm1, %xmm0
  vsubsd %xmm2, %xmm0, %xmm0
  vcomisd %xmm1, %xmm0
  jbe .L49
  vsubsd .LC4(%rip), %xmm0, %xmm0
  ret
.L48:
  vsubsd %xmm2, %xmm1, %xmm0
  vaddsd %xmm2, %xmm0, %xmm0
  vcomisd %xmm0, %xmm1
  jbe .L50
  vaddsd .LC4(%rip), %xmm0, %xmm0
  ret
.L49:
  ret
.L50:
  ret
copysign:
  vmovq %xmm0, %rax
  vmovq %xmm1, %rcx
  movabsq $-9223372036854775808, %rdx
  btrq $63, %rax
  andq %rcx, %rdx
  orq %rdx, %rax
  vmovq %rax, %xmm0
  ret
fma:
  vfmadd132sd %xmm1, %xmm2, %xmm0
  ret
.LC13:
main:
  leaq 8(%rsp), %r10
  andq $-32, %rsp
  movl $8, %ecx
  movl $buf, %eax
  pushq -8(%r10)
  vpcmpeqd %ymm7, %ymm7, %ymm7
  vmovd %ecx, %xmm8
  movl $buf+32768, %edx
  pushq %rbp
  vpsrld $30, %ymm7, %ymm9
  vpslld $11, %ymm7, %ymm7
  vpbroadcastd %xmm8, %ymm8
  movq %rsp, %rbp
  pushq %r10
  subq $8, %rsp
  vmovdqa .LC5(%rip), %ymm4
  vbroadcastsd .LC9(%rip), %ymm6
  vbroadcastsd .LC11(%rip), %ymm5
.L54:
  vpaddd %ymm7, %ymm4, %ymm1
  vpand %ymm9, %ymm4, %ymm0
  vpaddd %ymm8, %ymm4, %ymm4
  addq $64, %rax
  vextracti128 $0x1, %ymm1, %xmm3
  vcvtdq2pd %xmm1, %ymm1
  vextracti128 $0x1, %ymm0, %xmm2
  vcvtdq2pd %xmm0, %ymm0
  vmulpd %ymm6, %ymm1, %ymm1
  vcvtdq2pd %xmm3, %ymm3
  vcvtdq2pd %xmm2, %ymm2
  vmulpd %ymm6, %ymm3, %ymm3
  vfmadd132pd %ymm5, %ymm1, %ymm0
  vfmadd132pd %ymm5, %ymm3, %ymm2
  vmovapd %ymm0, -64(%rax)
  vmovapd %ymm2, -32(%rax)
  cmpq %rax, %rdx
  jne .L54
  xorl %ecx, %ecx
  vxorpd %xmm11, %xmm11, %xmm11
.L55:
  call round_family_pass
  movl %ecx, %eax
  addl $1, %ecx
  andl $4095, %eax
  vaddsd %xmm0, %xmm11, %xmm11
  vmovsd buf(,%rax,8), %xmm0
  vxorpd .LC1(%rip), %xmm0, %xmm0
  vmovsd %xmm0, buf(,%rax,8)
  cmpl $20000, %ecx
  jne .L55
  vmovapd %xmm11, %xmm0
  movl $.LC13, %edi
  movl $1, %eax
  vzeroupper
  call printf
  addq $8, %rsp
  xorl %eax, %eax
  popq %r10
  popq %rbp
  leaq -8(%r10), %rsp
  ret
.LC1:
.LC2:
.LC3:
.LC4:
.LC5:
.LC9:
.LC11:
