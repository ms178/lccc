.LC4:
main:
  pushq %rbp
  vmovdqa .LC0(%rip), %ymm3
  movl $img, %edx
  xorl %eax, %eax
  vmovdqa .LC3(%rip), %ymm2
  vmovdqa .LC1(%rip), %ymm6
  movl $3, %ecx
  vmovdqa .LC2(%rip), %ymm5
  vpunpcklbw %ymm3, %ymm3, %ymm8
  vpunpckhbw %ymm3, %ymm3, %ymm3
  movq %rsp, %rbp
  vpunpcklbw %ymm2, %ymm2, %ymm7
  vpunpckhbw %ymm2, %ymm2, %ymm2
  andq $-32, %rsp
.L2:
  vmovd %ecx, %xmm0
  vmovd %eax, %xmm4
  addl $5, %eax
  addl $1, %ecx
  vpbroadcastb %xmm0, %ymm0
  vpbroadcastb %xmm4, %ymm4
  addq $64, %rdx
  vpunpcklbw %ymm0, %ymm0, %ymm9
  vpunpckhbw %ymm0, %ymm0, %ymm0
  vpmullw %ymm0, %ymm3, %ymm1
  vpmullw %ymm9, %ymm8, %ymm10
  vpmullw %ymm0, %ymm2, %ymm0
  vpshufb %ymm6, %ymm10, %ymm10
  vpshufb %ymm5, %ymm1, %ymm1
  vpblendd $51, %ymm10, %ymm1, %ymm1
  vpshufb %ymm5, %ymm0, %ymm0
  vpaddb %ymm1, %ymm4, %ymm1
  vmovdqa %ymm1, -64(%rdx)
  vpmullw %ymm9, %ymm7, %ymm1
  vpshufb %ymm6, %ymm1, %ymm1
  vpblendd $51, %ymm1, %ymm0, %ymm0
  vpaddb %ymm0, %ymm4, %ymm4
  vmovdqa %ymm4, -32(%rdx)
  cmpb $64, %al
  jne .L2
  movl $62, %r9d
  movl $1, %r10d
.L3:
  leaq -62(%r9), %r8
.L9:
  movq %r8, %rdi
  xorl %esi, %esi
  xorl %ecx, %ecx
.L7:
  xorl %eax, %eax
.L4:
  movzbl img(%rdi,%rax), %edx
  imull k(%rsi,%rax,4), %edx
  addq $1, %rax
  addl %edx, %ecx
  cmpq $3, %rax
  jne .L4
  addq $12, %rsi
  addq $64, %rdi
  cmpq $36, %rsi
  jne .L7
  movl %ecx, %eax
  movl $255, %edx
  negl %eax
  cmovs %ecx, %eax
  cmpl %edx, %eax
  cmovg %edx, %eax
  addq $1, %r8
  movb %al, out+64(%r8)
  cmpq %r9, %r8
  jne .L9
  addl $1, %r10d
  leaq 64(%r8), %r9
  cmpl $63, %r10d
  jne .L3
  movl $out+64, %ecx
  xorl %esi, %esi
.L8:
  leaq -64(%rcx), %rax
.L10:
  movq %rsi, %rdx
  addq $1, %rax
  salq $5, %rdx
  subq %rsi, %rdx
  movzbl -1(%rax), %esi
  addq %rdx, %rsi
  cmpq %rax, %rcx
  jne .L10
  addq $64, %rcx
  cmpq $out+4160, %rcx
  jne .L8
  xorl %eax, %eax
  movl $.LC4, %edi
  vzeroupper
  call printf
  xorl %eax, %eax
  leave
  ret
k:
.LC0:
.LC1:
.LC2:
.LC3:
