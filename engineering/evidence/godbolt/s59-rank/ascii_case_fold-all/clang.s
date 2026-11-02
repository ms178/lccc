.LCPI0_0:
.LCPI0_1:
.LCPI0_2:
.LCPI0_3:
.LCPI0_4:
.LCPI0_5:
.LCPI0_9:
.LCPI0_10:
.LCPI0_11:
.LCPI0_12:
main:
  vmovdqa .LCPI0_0(%rip), %ymm0
  vmovdqa .LCPI0_1(%rip), %ymm1
  vmovdqa .LCPI0_2(%rip), %ymm2
  vmovdqa .LCPI0_3(%rip), %ymm3
  xorl %ecx, %ecx
  vpbroadcastd .LCPI0_4(%rip), %ymm4
  vpbroadcastd .LCPI0_5(%rip), %ymm5
  vpbroadcastd .LCPI0_10(%rip), %ymm6
  vpbroadcastd .LCPI0_11(%rip), %ymm7
  vpbroadcastd .LCPI0_12(%rip), %ymm8
  leaq data(%rip), %rax
  vpbroadcastd .LCPI0_9(%rip), %ymm9
.LBB0_1:
  vpmulld %ymm4, %ymm3, %ymm10
  vpmulld %ymm4, %ymm2, %ymm11
  vpmulld %ymm4, %ymm1, %ymm12
  vpmulld %ymm4, %ymm0, %ymm13
  vpaddd %ymm5, %ymm13, %ymm13
  vpaddd %ymm5, %ymm12, %ymm12
  vpaddd %ymm5, %ymm11, %ymm11
  vpaddd %ymm5, %ymm10, %ymm10
  vpsrld $16, %ymm10, %ymm10
  vpsrld $16, %ymm11, %ymm11
  vpackusdw %ymm11, %ymm10, %ymm10
  vpsrld $16, %ymm12, %ymm11
  vpsrld $16, %ymm13, %ymm12
  vpackusdw %ymm12, %ymm11, %ymm11
  vpermq $216, %ymm11, %ymm11
  vpermq $216, %ymm10, %ymm10
  vpmulhuw %ymm6, %ymm10, %ymm12
  vpsrlw $6, %ymm12, %ymm12
  vpmullw %ymm7, %ymm12, %ymm12
  vpsubw %ymm12, %ymm10, %ymm10
  vpmulhuw %ymm6, %ymm11, %ymm12
  vpsrlw $6, %ymm12, %ymm12
  vpmullw %ymm7, %ymm12, %ymm12
  vpsubw %ymm12, %ymm11, %ymm11
  vpackuswb %ymm11, %ymm10, %ymm10
  vpermq $216, %ymm10, %ymm10
  vpaddb %ymm8, %ymm10, %ymm10
  vmovdqu %ymm10, (%rcx,%rax)
  addq $32, %rcx
  vpaddd %ymm3, %ymm9, %ymm3
  vpaddd %ymm2, %ymm9, %ymm2
  vpaddd %ymm1, %ymm9, %ymm1
  vpaddd %ymm0, %ymm9, %ymm0
  cmpq $65536, %rcx
  jne .LBB0_1
  xorl %edi, %edi
  leaq folded(%rip), %rcx
  xorl %edx, %edx
.LBB0_3:
  movzbl (%rdx,%rax), %esi
  leal -65(%rsi), %r8d
  leal 32(%rsi), %r9d
  cmpb $26, %r8b
  cmovael %esi, %r9d
  movb %r9b, (%rdx,%rcx)
  movl %edi, %esi
  shll $5, %esi
  addl %edi, %esi
  addl %r9d, %esi
  movzbl 1(%rdx,%rax), %edi
  leal -65(%rdi), %r8d
  leal 32(%rdi), %r9d
  cmpb $26, %r8b
  cmovael %edi, %r9d
  movb %r9b, 1(%rdx,%rcx)
  addl %esi, %r9d
  shll $5, %esi
  addl %r9d, %esi
  movzbl 2(%rdx,%rax), %edi
  leal -65(%rdi), %r8d
  leal 32(%rdi), %r9d
  cmpb $26, %r8b
  cmovael %edi, %r9d
  movb %r9b, 2(%rdx,%rcx)
  addl %esi, %r9d
  shll $5, %esi
  addl %r9d, %esi
  movzbl 3(%rdx,%rax), %edi
  leal -65(%rdi), %r8d
  leal 32(%rdi), %r9d
  cmpb $26, %r8b
  cmovael %edi, %r9d
  movb %r9b, 3(%rdx,%rcx)
  addl %esi, %r9d
  shll $5, %esi
  movl %esi, %edi
  addl %r9d, %edi
  addq $4, %rdx
  cmpq $65536, %rdx
  jne .LBB0_3
  movzbl folded(%rip), %eax
  xorb $32, %al
  movzbl folded+1(%rip), %ecx
  xorb $55, %cl
  orb %al, %cl
  movzbl folded+2(%rip), %eax
  xorb $110, %al
  xorl %edx, %edx
  cmpl $-1723667183, %edi
  setne %dl
  addl %edx, %edx
  orb %cl, %al
  movl $1, %eax
  cmovel %edx, %eax
  vzeroupper
  retq

