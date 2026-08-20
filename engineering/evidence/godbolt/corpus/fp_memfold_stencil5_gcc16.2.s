stencil5.constprop.0:
  xorl %eax, %eax
.L2:
  vmovups input+4(%rax), %ymm0
  vaddps input(%rax), %ymm0, %ymm0
  addq $32, %rax
  vaddps input-24(%rax), %ymm0, %ymm0
  vaddps input-20(%rax), %ymm0, %ymm0
  vaddps input-16(%rax), %ymm0, %ymm0
  vmovups %ymm0, output-24(%rax)
  cmpq $262112, %rax
  jne .L2
  vmovss input+262116(%rip), %xmm0
  vmovss input+262120(%rip), %xmm2
  xorl %eax, %eax
  vmovss input+262124(%rip), %xmm1
.L3:
  vaddss input+262112(%rax), %xmm0, %xmm0
  vmovaps %xmm1, %xmm3
  addq $4, %rax
  vaddss %xmm2, %xmm0, %xmm0
  vaddss %xmm1, %xmm0, %xmm0
  vmovss input+262124(%rax), %xmm1
  vaddss %xmm0, %xmm1, %xmm0
  vmovss %xmm0, output+262116(%rax)
  vmovaps %xmm2, %xmm0
  vmovaps %xmm3, %xmm2
  cmpq $16, %rax
  jne .L3
  vzeroupper
  ret
.LC6:
  .string "%.0f\n"
main:
  leaq 8(%rsp), %r10
  andq $-32, %rsp
  pushq -8(%r10)
  pushq %rbp
  movq %rsp, %rbp
  pushq %r10
  subq $40, %rsp
  cmpl $1, %edi
  jg .L29
  movl $1000, %esi
.L8:
  movl $8, %edx
  vpcmpeqd %ymm3, %ymm3, %ymm3
  vmovdqa .LC0(%rip), %ymm1
  vbroadcastss .LC4(%rip), %ymm4
  vmovd %edx, %xmm2
  movl $input, %eax
  movl $input+262144, %ecx
  vpsrld $28, %ymm3, %ymm3
  vpbroadcastd %xmm2, %ymm2
.L9:
  vpand %ymm3, %ymm1, %ymm0
  addq $32, %rax
  vpaddd %ymm2, %ymm1, %ymm1
  vcvtdq2ps %ymm0, %ymm0
  vmulps %ymm4, %ymm0, %ymm0
  vmovaps %ymm0, -32(%rax)
  cmpq %rax, %rcx
  jne .L9
  testl %esi, %esi
  jle .L10
  xorl %edx, %edx
  testb $1, %sil
  jne .L25
  vzeroupper
.L11:
  call stencil5.constprop.0
  addl $2, %edx
  call stencil5.constprop.0
  cmpl %edx, %esi
  jne .L11
.L10:
  movl $output, %eax
  movl $output+262144, %edx
  vxorpd %xmm4, %xmm4, %xmm4
.L12:
  vmovaps (%rax), %ymm1
  addq $32, %rax
  vcvtps2pd %xmm1, %ymm2
  vaddsd %xmm4, %xmm2, %xmm0
  vunpckhpd %xmm2, %xmm2, %xmm3
  vextractf128 $0x1, %ymm2, %xmm2
  vextractf128 $0x1, %ymm1, %xmm1
  vcvtps2pd %xmm1, %ymm1
  vaddsd %xmm3, %xmm0, %xmm0
  vaddsd %xmm2, %xmm0, %xmm0
  vunpckhpd %xmm2, %xmm2, %xmm2
  vaddsd %xmm2, %xmm0, %xmm0
  vunpckhpd %xmm1, %xmm1, %xmm2
  vaddsd %xmm1, %xmm0, %xmm0
  vextractf128 $0x1, %ymm1, %xmm1
  vaddsd %xmm2, %xmm0, %xmm0
  vaddsd %xmm1, %xmm0, %xmm0
  vunpckhpd %xmm1, %xmm1, %xmm1
  vaddsd %xmm1, %xmm0, %xmm4
  cmpq %rax, %rdx
  jne .L12
  vmovapd %xmm4, %xmm0
  vmovsd %xmm4, -24(%rbp)
  movl $.LC6, %edi
  movl $1, %eax
  vzeroupper
  call printf
  vmovsd -24(%rbp), %xmm4
  vcomisd .LC7(%rip), %xmm4
  movl $1, %eax
  jbe .L7
  vmovsd .LC8(%rip), %xmm0
  xorl %eax, %eax
  vcomisd %xmm4, %xmm0
  setbe %al
.L7:
  addq $40, %rsp
  popq %r10
  popq %rbp
  leaq -8(%r10), %rsp
  ret
.L25:
  vzeroupper
  call stencil5.constprop.0
  movl $1, %edx
  cmpl $1, %esi
  jne .L11
  jmp .L10
.L29:
  movq 8(%rsi), %rdi
  movl $10, %edx
  xorl %esi, %esi
  call __isoc23_strtol
  movl %eax, %esi
  jmp .L8
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
  .long 1048576000
.LC7:
  .long 0
  .long 1092767616
.LC8:
  .long 0
  .long 1092807616
