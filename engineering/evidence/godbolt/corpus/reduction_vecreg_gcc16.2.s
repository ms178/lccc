dot_f32.constprop.0:
  xorl %eax, %eax
  vxorps %xmm0, %xmm0, %xmm0
.L2:
  vmovaps input_a(%rax), %ymm1
  vmulps input_b(%rax), %ymm1, %ymm1
  addq $32, %rax
  vaddss %xmm1, %xmm0, %xmm0
  vshufps $85, %xmm1, %xmm1, %xmm3
  vshufps $255, %xmm1, %xmm1, %xmm2
  vaddss %xmm3, %xmm0, %xmm0
  vunpckhps %xmm1, %xmm1, %xmm3
  vextractf128 $0x1, %ymm1, %xmm1
  vaddss %xmm3, %xmm0, %xmm0
  vaddss %xmm2, %xmm0, %xmm0
  vshufps $85, %xmm1, %xmm1, %xmm2
  vaddss %xmm1, %xmm0, %xmm0
  vaddss %xmm2, %xmm0, %xmm0
  vunpckhps %xmm1, %xmm1, %xmm2
  vshufps $255, %xmm1, %xmm1, %xmm1
  vaddss %xmm2, %xmm0, %xmm0
  vaddss %xmm1, %xmm0, %xmm0
  cmpq $262144, %rax
  jne .L2
  vzeroupper
  ret
sum_f32.constprop.0:
  movl $input_a, %eax
  movl $input_a+262144, %edx
  vxorps %xmm0, %xmm0, %xmm0
.L6:
  vaddss (%rax), %xmm0, %xmm0
  addq $32, %rax
  vaddss -28(%rax), %xmm0, %xmm0
  vaddss -24(%rax), %xmm0, %xmm0
  vaddss -20(%rax), %xmm0, %xmm0
  vaddss -16(%rax), %xmm0, %xmm0
  vaddss -12(%rax), %xmm0, %xmm0
  vaddss -8(%rax), %xmm0, %xmm0
  vaddss -4(%rax), %xmm0, %xmm0
  cmpq %rax, %rdx
  jne .L6
  ret
.LC9:
  .string "%.0f\n"
main:
  leaq 8(%rsp), %r10
  andq $-32, %rsp
  pushq -8(%r10)
  pushq %rbp
  movq %rsp, %rbp
  pushq %r10
  subq $8, %rsp
  cmpl $1, %edi
  jg .L19
  movl $5000, %esi
.L9:
  movl $8, %edx
  vpcmpeqd %ymm2, %ymm2, %ymm2
  xorl %eax, %eax
  vmovdqa .LC1(%rip), %ymm1
  vmovd %edx, %xmm3
  vpsrld $28, %ymm2, %ymm4
  vbroadcastss .LC4(%rip), %ymm6
  vbroadcastss .LC7(%rip), %ymm5
  vpsrld $29, %ymm2, %ymm2
  vpbroadcastd %xmm3, %ymm3
.L10:
  vpand %ymm4, %ymm1, %ymm0
  addq $32, %rax
  vcvtdq2ps %ymm0, %ymm0
  vmulps %ymm6, %ymm0, %ymm0
  vmovaps %ymm0, input_a-32(%rax)
  vpand %ymm2, %ymm1, %ymm0
  vpaddd %ymm3, %ymm1, %ymm1
  vcvtdq2ps %ymm0, %ymm0
  vmulps %ymm5, %ymm0, %ymm0
  vmovaps %ymm0, input_b-32(%rax)
  cmpq $262144, %rax
  jne .L10
  testl %esi, %esi
  jle .L16
  xorl %ecx, %ecx
  vzeroupper
.L12:
  call sum_f32.constprop.0
  addl $1, %ecx
  vmovss %xmm0, sink(%rip)
  call dot_f32.constprop.0
  vaddss sink(%rip), %xmm0, %xmm0
  vmovss %xmm0, sink(%rip)
  cmpl %ecx, %esi
  jne .L12
.L11:
  vxorps %xmm0, %xmm0, %xmm0
  movl $.LC9, %edi
  movl $1, %eax
  vcvtss2sd sink(%rip), %xmm0, %xmm0
  call printf
  xorl %eax, %eax
  vmovss sink(%rip), %xmm0
  vucomiss .LC10(%rip), %xmm0
  movl $1, %edx
  setp %al
  cmovne %edx, %eax
  addq $8, %rsp
  popq %r10
  popq %rbp
  leaq -8(%r10), %rsp
  ret
.L19:
  movq 8(%rsi), %rdi
  movl $10, %edx
  xorl %esi, %esi
  call __isoc23_strtol
  movl %eax, %esi
  jmp .L9
.L16:
  vzeroupper
  jmp .L11
.LC1:
  .long 0
  .long 1
  .long 2
  .long 3
  .long 4
  .long 5
  .long 6
  .long 7
.LC4:
  .long 1048576000
.LC7:
  .long 1040187392
.LC10:
  .long 1211564032
