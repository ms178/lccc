mul_Atv.constprop.0:
  movl $8, %eax
  vpcmpeqd %ymm3, %ymm3, %ymm3
  vbroadcastsd .LC4(%rip), %ymm4
  vmovdqa .LC0(%rip), %ymm7
  vmovd %eax, %xmm6
  xorl %ecx, %ecx
  leaq 16000(%rdi), %rdx
  vpsrld $31, %ymm3, %ymm3
  vpbroadcastd %xmm6, %ymm6
.L2:
  vmovd %ecx, %xmm5
  vmovdqa %ymm7, %ymm2
  vxorpd %xmm8, %xmm8, %xmm8
  movq %rdi, %rax
  vpbroadcastd %xmm5, %ymm5
.L3:
  vpaddd %ymm5, %ymm2, %ymm1
  addq $64, %rax
  vpaddd %ymm3, %ymm1, %ymm0
  vpmulld %ymm1, %ymm0, %ymm0
  vpsrad $1, %ymm0, %ymm0
  vpaddd %ymm2, %ymm0, %ymm0
  vpaddd %ymm6, %ymm2, %ymm2
  vpaddd %ymm3, %ymm0, %ymm0
  vcvtdq2pd %xmm0, %ymm9
  vdivpd %ymm9, %ymm4, %ymm9
  vextracti128 $0x1, %ymm0, %xmm0
  vcvtdq2pd %xmm0, %ymm0
  vdivpd %ymm0, %ymm4, %ymm0
  vmulpd -64(%rax), %ymm9, %ymm9
  vaddsd %xmm9, %xmm8, %xmm1
  vunpckhpd %xmm9, %xmm9, %xmm10
  vextractf128 $0x1, %ymm9, %xmm8
  vaddsd %xmm10, %xmm1, %xmm1
  vaddsd %xmm8, %xmm1, %xmm1
  vunpckhpd %xmm8, %xmm8, %xmm8
  vaddsd %xmm8, %xmm1, %xmm1
  vmulpd -32(%rax), %ymm0, %ymm0
  vaddsd %xmm0, %xmm1, %xmm1
  vunpckhpd %xmm0, %xmm0, %xmm8
  vextractf128 $0x1, %ymm0, %xmm0
  vaddsd %xmm8, %xmm1, %xmm1
  vaddsd %xmm0, %xmm1, %xmm1
  vunpckhpd %xmm0, %xmm0, %xmm0
  vaddsd %xmm0, %xmm1, %xmm8
  cmpq %rax, %rdx
  jne .L3
  vmovsd %xmm8, (%rsi,%rcx,8)
  addq $1, %rcx
  cmpq $2000, %rcx
  jne .L2
  vzeroupper
  ret
mul_Av.constprop.0:
  movl $8, %eax
  vpcmpeqd %ymm4, %ymm4, %ymm4
  vbroadcastsd .LC4(%rip), %ymm5
  vmovdqa .LC0(%rip), %ymm7
  vmovd %eax, %xmm6
  xorl %ecx, %ecx
  leaq 16000(%rdi), %rdx
  vpsrld $31, %ymm4, %ymm4
  vpbroadcastd %xmm6, %ymm6
.L9:
  vmovd %ecx, %xmm3
  vmovdqa %ymm7, %ymm2
  vxorpd %xmm8, %xmm8, %xmm8
  movq %rdi, %rax
  vpbroadcastd %xmm3, %ymm3
.L10:
  vpaddd %ymm3, %ymm2, %ymm1
  addq $64, %rax
  vpaddd %ymm6, %ymm2, %ymm2
  vpaddd %ymm4, %ymm1, %ymm0
  vpmulld %ymm1, %ymm0, %ymm0
  vpsrad $1, %ymm0, %ymm0
  vpaddd %ymm3, %ymm0, %ymm0
  vpaddd %ymm4, %ymm0, %ymm0
  vcvtdq2pd %xmm0, %ymm9
  vdivpd %ymm9, %ymm5, %ymm9
  vextracti128 $0x1, %ymm0, %xmm0
  vcvtdq2pd %xmm0, %ymm0
  vdivpd %ymm0, %ymm5, %ymm0
  vmulpd -64(%rax), %ymm9, %ymm9
  vaddsd %xmm9, %xmm8, %xmm1
  vunpckhpd %xmm9, %xmm9, %xmm10
  vextractf128 $0x1, %ymm9, %xmm8
  vaddsd %xmm10, %xmm1, %xmm1
  vaddsd %xmm8, %xmm1, %xmm1
  vunpckhpd %xmm8, %xmm8, %xmm8
  vaddsd %xmm8, %xmm1, %xmm1
  vmulpd -32(%rax), %ymm0, %ymm0
  vaddsd %xmm0, %xmm1, %xmm1
  vunpckhpd %xmm0, %xmm0, %xmm8
  vextractf128 $0x1, %ymm0, %xmm0
  vaddsd %xmm8, %xmm1, %xmm1
  vaddsd %xmm0, %xmm1, %xmm1
  vunpckhpd %xmm0, %xmm0, %xmm0
  vaddsd %xmm0, %xmm1, %xmm8
  cmpq %rax, %rdx
  jne .L10
  vmovsd %xmm8, (%rsi,%rcx,8)
  addq $1, %rcx
  cmpq $2000, %rcx
  jne .L9
  vzeroupper
  ret
.LC6:
  .string "%.9f\n"
main:
  leaq 8(%rsp), %r10
  andq $-32, %rsp
  pushq -8(%r10)
  pushq %rbp
  movq %rsp, %rbp
  pushq %r10
  leaq -48016(%rbp), %rax
  leaq -32016(%rbp), %r8
  subq $48008, %rsp
  vbroadcastsd .LC4(%rip), %ymm0
.L15:
  vmovapd %ymm0, (%rax)
  addq $64, %rax
  vmovapd %ymm0, -32(%rax)
  cmpq %r8, %rax
  jne .L15
  movl $10, %r9d
  vzeroupper
.L16:
  leaq -16016(%rbp), %rsi
  leaq -48016(%rbp), %rdi
  call mul_Av.constprop.0
  leaq -16016(%rbp), %rdi
  movq %r8, %rsi
  call mul_Atv.constprop.0
  movq %rdi, %rsi
  movq %r8, %rdi
  call mul_Av.constprop.0
  leaq -48016(%rbp), %rsi
  leaq -16016(%rbp), %rdi
  call mul_Atv.constprop.0
  subl $1, %r9d
  jne .L16
  vxorpd %xmm4, %xmm4, %xmm4
  xorl %eax, %eax
  vmovapd %xmm4, %xmm1
.L17:
  vmovapd (%r8,%rax), %ymm0
  vmulpd -48016(%rbp,%rax), %ymm0, %ymm2
  addq $32, %rax
  vmulpd %ymm0, %ymm0, %ymm0
  vaddsd %xmm2, %xmm1, %xmm1
  vunpckhpd %xmm2, %xmm2, %xmm3
  vextractf128 $0x1, %ymm2, %xmm2
  vaddsd %xmm4, %xmm0, %xmm4
  vaddsd %xmm3, %xmm1, %xmm1
  vunpckhpd %xmm0, %xmm0, %xmm3
  vextractf128 $0x1, %ymm0, %xmm0
  vaddsd %xmm4, %xmm3, %xmm3
  vaddsd %xmm2, %xmm1, %xmm1
  vunpckhpd %xmm2, %xmm2, %xmm2
  vaddsd %xmm3, %xmm0, %xmm4
  vunpckhpd %xmm0, %xmm0, %xmm0
  vaddsd %xmm2, %xmm1, %xmm1
  vaddsd %xmm0, %xmm4, %xmm4
  cmpq $16000, %rax
  jne .L17
  vdivsd %xmm4, %xmm1, %xmm0
  vxorpd %xmm1, %xmm1, %xmm1
  vucomisd %xmm0, %xmm1
  ja .L26
  vsqrtsd %xmm0, %xmm0, %xmm0
  vzeroupper
.L20:
  movl $.LC6, %edi
  movl $1, %eax
  call printf
  addq $48008, %rsp
  xorl %eax, %eax
  popq %r10
  popq %rbp
  leaq -8(%r10), %rsp
  ret
.L26:
  vzeroupper
  call sqrt
  jmp .L20
.LC0:
  .long 0
  .long 1
  .long 2
  .long 3
  .long 4
  .long 5
  .long 6
  .long 7
.LC4:
  .long 0
  .long 1072693248
