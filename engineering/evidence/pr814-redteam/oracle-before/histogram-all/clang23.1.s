.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
.LCPI0_3:
.LCPI0_4:
.LCPI0_5:
.LCPI0_6:
main:
  vmovdqa .LCPI0_0(%rip), %ymm0
  vmovdqa .LCPI0_1(%rip), %ymm1
  vmovdqa .LCPI0_2(%rip), %ymm2
  vmovdqa .LCPI0_3(%rip), %ymm3
  xorl %eax, %eax
  vpbroadcastd .LCPI0_4(%rip), %ymm4
  leaq bytes(%rip), %rcx
  vpbroadcastd .LCPI0_5(%rip), %ymm5
  vpbroadcastd .LCPI0_6(%rip), %ymm6
.LBB0_1:
  vpmulld %ymm4, %ymm2, %ymm7
  vpmulld %ymm4, %ymm3, %ymm8
  vpmulld %ymm4, %ymm0, %ymm9
  vpmulld %ymm4, %ymm1, %ymm10
  vpsrld $24, %ymm10, %ymm11
  vpsrld $24, %ymm9, %ymm12
  vpackusdw %ymm12, %ymm11, %ymm11
  vpsrld $24, %ymm8, %ymm12
  vpsrld $24, %ymm7, %ymm13
  vpackusdw %ymm13, %ymm12, %ymm12
  vpermq $216, %ymm12, %ymm12
  vpermq $216, %ymm11, %ymm11
  vpackuswb %ymm11, %ymm12, %ymm11
  vpermq $216, %ymm11, %ymm11
  vmovdqu %ymm11, (%rax,%rcx)
  vpaddd %ymm5, %ymm7, %ymm7
  vpaddd %ymm5, %ymm8, %ymm8
  vpaddd %ymm5, %ymm9, %ymm9
  vpaddd %ymm5, %ymm10, %ymm10
  vpsrld $24, %ymm10, %ymm10
  vpsrld $24, %ymm9, %ymm9
  vpackusdw %ymm9, %ymm10, %ymm9
  vpsrld $24, %ymm8, %ymm8
  vpsrld $24, %ymm7, %ymm7
  vpackusdw %ymm7, %ymm8, %ymm7
  vpermq $216, %ymm7, %ymm7
  vpermq $216, %ymm9, %ymm8
  vpackuswb %ymm8, %ymm7, %ymm7
  vpermq $216, %ymm7, %ymm7
  vmovdqu %ymm7, 32(%rax,%rcx)
  addq $64, %rax
  vpaddd %ymm6, %ymm3, %ymm3
  vpaddd %ymm6, %ymm2, %ymm2
  vpaddd %ymm6, %ymm1, %ymm1
  vpaddd %ymm6, %ymm0, %ymm0
  cmpq $262144, %rax
  jne .LBB0_1
  xorl %edx, %edx
  leaq bins(%rip), %rax
.LBB0_3:
  movzbl (%rdx,%rcx), %esi
  incq (%rax,%rsi,8)
  movzbl 1(%rdx,%rcx), %esi
  incq (%rax,%rsi,8)
  movzbl 2(%rdx,%rcx), %esi
  incq (%rax,%rsi,8)
  movzbl 3(%rdx,%rcx), %esi
  incq (%rax,%rsi,8)
  addq $4, %rdx
  cmpq $262144, %rdx
  jne .LBB0_3
  xorl %ecx, %ecx
  xorl %edi, %edi
  xorl %edx, %edx
.LBB0_5:
  movq (%rax,%rcx,8), %r8
  movq 8(%rax,%rcx,8), %rsi
  addq %r8, %rdx
  imulq $131, %rdi, %rdi
  addq %r8, %rdi
  imulq $131, %rdi, %rdi
  addq %rsi, %rdi
  movq 16(%rax,%rcx,8), %r8
  addq %r8, %rsi
  addq %rdx, %rsi
  imulq $131, %rdi, %rdx
  addq %r8, %rdx
  movq 24(%rax,%rcx,8), %rdi
  imulq $131, %rdx, %rdx
  addq %rdi, %rdx
  movq 32(%rax,%rcx,8), %r8
  addq %r8, %rdi
  imulq $131, %rdx, %rdx
  addq %r8, %rdx
  movq 40(%rax,%rcx,8), %r8
  addq %r8, %rdi
  addq %rsi, %rdi
  imulq $131, %rdx, %rsi
  addq %r8, %rsi
  movq 48(%rax,%rcx,8), %rdx
  imulq $131, %rsi, %rsi
  addq %rdx, %rsi
  movq 56(%rax,%rcx,8), %r8
  addq %r8, %rdx
  addq %rdi, %rdx
  imulq $131, %rsi, %rdi
  addq %r8, %rdi
  addq $8, %rcx
  cmpq $256, %rcx
  jne .LBB0_5
  movabsq $-7388273516099151790, %rax
  xorl %ecx, %ecx
  cmpq %rax, %rdi
  setne %cl
  addl %ecx, %ecx
  cmpq $262144, %rdx
  movl $1, %eax
  cmovel %ecx, %eax
  vzeroupper
  retq

