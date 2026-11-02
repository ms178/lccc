.LC4:
main:
  pushq %rbp
  movl $8, %edx
  vpcmpeqd %ymm5, %ymm5, %ymm5
  movl $a.1, %eax
  vmovd %edx, %xmm6
  movl $16, %edx
  vpxor %xmm3, %xmm3, %xmm3
  vmovdqa .LC0(%rip), %ymm2
  vmovd %edx, %xmm4
  vpsrlw $13, %ymm5, %ymm5
  vpbroadcastd %xmm6, %ymm6
  movq %rsp, %rbp
  vpbroadcastd %xmm4, %ymm4
  andq $-32, %rsp
.L2:
  vpaddd %ymm6, %ymm2, %ymm1
  vpblendw $170, %ymm3, %ymm2, %ymm0
  addq $32, %rax
  vpblendw $170, %ymm3, %ymm1, %ymm1
  vpaddd %ymm4, %ymm2, %ymm2
  vpackusdw %ymm1, %ymm0, %ymm0
  vpermq $216, %ymm0, %ymm0
  vpsllw $5, %ymm0, %ymm1
  vpsubw %ymm0, %ymm1, %ymm0
  vpaddw %ymm5, %ymm0, %ymm0
  vmovdqa %ymm0, -32(%rax)
  cmpq $a.1+2048, %rax
  jne .L2
  xorl %eax, %eax
  xorl %edx, %edx
.L3:
  movzwl a.1(%rax,%rax), %ecx
  addl %ecx, %edx
  movl %edx, d.0(,%rax,4)
  addq $1, %rax
  cmpq $1024, %rax
  jne .L3
  movl d.0+2044(%rip), %esi
  movl $d.0, %eax
  addl d.0(%rip), %esi
  movl $d.0+4116, %ecx
  addl d.0+4092(%rip), %esi
.L4:
  movq %rsi, %rdx
  addq $28, %rax
  salq $5, %rdx
  addq %rdx, %rsi
  movl -28(%rax), %edx
  addq %rdx, %rsi
  cmpq %rax, %rcx
  jne .L4
  xorl %eax, %eax
  movl $.LC4, %edi
  vzeroupper
  call printf
  xorl %eax, %eax
  leave
  ret
.LC0:
