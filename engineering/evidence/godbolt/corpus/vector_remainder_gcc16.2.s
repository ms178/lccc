dot_f64.constprop.0:
  testl %edi, %edi
  je .L6
  leal -1(%rdi), %eax
  cmpl $2, %eax
  jbe .L7
  movl %edi, %edx
  xorl %eax, %eax
  vxorpd %xmm0, %xmm0, %xmm0
  shrl $2, %edx
  movl %edx, %ecx
  salq $5, %rcx
.L4:
  vmovapd a(%rax), %ymm1
  vmulpd b(%rax), %ymm1, %ymm1
  addq $32, %rax
  vaddsd %xmm1, %xmm0, %xmm0
  vunpckhpd %xmm1, %xmm1, %xmm2
  vextractf128 $0x1, %ymm1, %xmm1
  vaddsd %xmm2, %xmm0, %xmm0
  vaddsd %xmm1, %xmm0, %xmm0
  vunpckhpd %xmm1, %xmm1, %xmm1
  vaddsd %xmm1, %xmm0, %xmm0
  cmpq %rcx, %rax
  jne .L4
  leal 0(,%rdx,4), %eax
  cmpl %eax, %edi
  je .L10
  vzeroupper
.L5:
  vmovsd a(,%rax,8), %xmm1
  vmulsd b(,%rax,8), %xmm1, %xmm1
  addq $1, %rax
  vaddsd %xmm1, %xmm0, %xmm0
  cmpl %eax, %edi
  jg .L5
  ret
.L10:
  vzeroupper
  ret
.L6:
  vxorpd %xmm0, %xmm0, %xmm0
  ret
.L7:
  vxorpd %xmm0, %xmm0, %xmm0
  xorl %eax, %eax
  jmp .L5
sum_f64.constprop.0:
  testl %edi, %edi
  je .L15
  movl %edi, %edi
  movl $a, %eax
  vxorpd %xmm0, %xmm0, %xmm0
  leaq a(,%rdi,8), %rdx
  andl $1, %edi
  je .L14
  movl $a+8, %eax
  vaddsd a(%rip), %xmm0, %xmm0
  cmpq %rdx, %rax
  je .L22
.L14:
  vaddsd (%rax), %xmm0, %xmm0
  addq $16, %rax
  vaddsd -8(%rax), %xmm0, %xmm0
  cmpq %rdx, %rax
  jne .L14
  ret
.L15:
  vxorpd %xmm0, %xmm0, %xmm0
  ret
.L22:
  ret
.LC7:
  .string "%.0f\n"
main:
  leaq 8(%rsp), %r10
  andq $-32, %rsp
  movl $1, %r9d
  pushq -8(%r10)
  pushq %rbp
  movq %rsp, %rbp
  pushq %r10
  subq $40, %rsp
  cmpl $1, %edi
  jle .L24
  movq 8(%rsi), %rdi
  movl $10, %edx
  xorl %esi, %esi
  call __isoc23_strtol
  movl %eax, %r9d
.L24:
  movl $2, %edx
  vpcmpeqd %ymm5, %ymm5, %ymm5
  vmovdqa .LC1(%rip), %ymm2
  xorl %eax, %eax
  vmovd %edx, %xmm6
  vpsrld $31, %ymm5, %ymm5
  movl $8, %edx
  vmovd %edx, %xmm4
  vpbroadcastd %xmm6, %ymm6
  vpbroadcastd %xmm4, %ymm4
.L25:
  vpaddd %ymm5, %ymm2, %ymm1
  vpaddd %ymm6, %ymm2, %ymm0
  addq $64, %rax
  vcvtdq2pd %xmm1, %ymm3
  vextracti128 $0x1, %ymm1, %xmm1
  vpaddd %ymm4, %ymm2, %ymm2
  vmovapd %ymm3, a-64(%rax)
  vcvtdq2pd %xmm1, %ymm1
  vmovapd %ymm1, a-32(%rax)
  vcvtdq2pd %xmm0, %ymm1
  vextracti128 $0x1, %ymm0, %xmm0
  vcvtdq2pd %xmm0, %ymm0
  vmovapd %ymm1, b-64(%rax)
  vmovapd %ymm0, b-32(%rax)
  cmpq $512, %rax
  jne .L25
  movq .LC5(%rip), %rax
  xorl %r8d, %r8d
  vxorpd %xmm1, %xmm1, %xmm1
  movq %rax, a+512(%rip)
  movq .LC6(%rip), %rax
  movq %rax, b+512(%rip)
  testl %r9d, %r9d
  jle .L37
  vzeroupper
.L26:
  movl $bounds.0, %esi
.L28:
  movl (%rsi), %ecx
  addq $4, %rsi
  movl %ecx, %edi
  call sum_f64.constprop.0
  movl %ecx, %edi
  vaddsd %xmm1, %xmm0, %xmm7
  vmovsd %xmm7, -24(%rbp)
  call dot_f64.constprop.0
  vaddsd -24(%rbp), %xmm0, %xmm1
  cmpq $bounds.0+72, %rsi
  jne .L28
  addl $1, %r8d
  cmpl %r8d, %r9d
  jne .L26
.L27:
  vmovapd %xmm1, %xmm0
  movl $.LC7, %edi
  movl $1, %eax
  call printf
  addq $40, %rsp
  xorl %eax, %eax
  popq %r10
  popq %rbp
  leaq -8(%r10), %rsp
  ret
.L37:
  vzeroupper
  jmp .L27
bounds.0:
  .long 0
  .long 1
  .long 2
  .long 3
  .long 4
  .long 5
  .long 7
  .long 8
  .long 9
  .long 15
  .long 16
  .long 17
  .long 31
  .long 32
  .long 33
  .long 63
  .long 64
  .long 65
.LC1:
  .long 0
  .long 1
  .long 2
  .long 3
  .long 4
  .long 5
  .long 6
  .long 7
.LC5:
  .long 0
  .long 1079001088
.LC6:
  .long 0
  .long 1079017472
