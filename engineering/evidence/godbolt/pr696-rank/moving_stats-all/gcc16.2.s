.LC8:
main:
  movl $717, %edx
  pushq %rbp
  movl $a.3, %eax
  vpxor %xmm8, %xmm8, %xmm8
  vmovd %edx, %xmm7
  movl $41, %edx
  vmovdqa .LC0(%rip), %ymm3
  vmovd %edx, %xmm5
  movl $8, %edx
  vpbroadcastd %xmm7, %ymm7
  vmovd %edx, %xmm11
  movl $1098962147, %edx
  movq %rsp, %rbp
  andq $-32, %rsp
  vmovd %edx, %xmm2
  movl $2001, %edx
  vpbroadcastd %xmm5, %ymm5
  vmovd %edx, %xmm4
  movl $-65471464, %edx
  vpbroadcastd %xmm11, %ymm11
  vmovd %edx, %xmm10
  movl $16, %edx
  vpbroadcastd %xmm2, %ymm2
  vmovd %edx, %xmm9
  vpbroadcastd %xmm4, %ymm4
  vpbroadcastd %xmm10, %ymm10
  vpbroadcastd %xmm9, %ymm9
.L2:
  vpmulld %ymm7, %ymm3, %ymm0
  vpaddd %ymm11, %ymm3, %ymm1
  addq $32, %rax
  vpmulld %ymm7, %ymm1, %ymm1
  vpaddd %ymm9, %ymm3, %ymm3
  vpaddd %ymm5, %ymm0, %ymm0
  vpmuldq %ymm2, %ymm0, %ymm13
  vpsrlq $32, %ymm0, %ymm12
  vpaddd %ymm5, %ymm1, %ymm1
  vpmuldq %ymm2, %ymm12, %ymm12
  vpshufd $245, %ymm13, %ymm13
  vpblendd $85, %ymm13, %ymm12, %ymm12
  vpmuldq %ymm2, %ymm1, %ymm13
  vpsrad $9, %ymm12, %ymm12
  vpmulld %ymm4, %ymm12, %ymm12
  vpshufd $245, %ymm13, %ymm13
  vpsubd %ymm12, %ymm0, %ymm0
  vpsrlq $32, %ymm1, %ymm12
  vpmuldq %ymm2, %ymm12, %ymm12
  vpblendw $170, %ymm8, %ymm0, %ymm0
  vpblendd $85, %ymm13, %ymm12, %ymm12
  vpsrad $9, %ymm12, %ymm12
  vpmulld %ymm4, %ymm12, %ymm12
  vpsubd %ymm12, %ymm1, %ymm1
  vpblendw $170, %ymm8, %ymm1, %ymm1
  vpackusdw %ymm1, %ymm0, %ymm0
  vpermq $216, %ymm0, %ymm0
  vpaddw %ymm0, %ymm10, %ymm0
  vmovdqa %ymm0, -32(%rax)
  cmpq $a.3+2048, %rax
  jne .L2
  movl $a.3+32, %edx
  movl $a.3, %esi
  xorl %ecx, %ecx
.L4:
  vmovdqa (%rsi), %ymm2
  movq %rsi, %rax
  vpxor %xmm4, %xmm4, %xmm4
  vpxor %xmm5, %xmm5, %xmm5
  vmovdqa %ymm2, %ymm3
.L3:
  vmovdqu (%rax), %ymm0
  addq $2, %rax
  vpmovsxwd %xmm0, %ymm1
  vpmaxsw %ymm0, %ymm2, %ymm2
  vpminsw %ymm0, %ymm3, %ymm3
  vextracti128 $0x1, %ymm0, %xmm0
  vpaddd %ymm1, %ymm5, %ymm5
  vpmovsxwd %xmm0, %ymm0
  vpaddd %ymm0, %ymm4, %ymm4
  cmpq %rax, %rdx
  jne .L3
  vmovdqa %ymm3, mns.1(%rcx)
  addq $32, %rsi
  addq $32, %rdx
  vmovdqa %ymm5, sums.2(%rcx,%rcx)
  vmovdqa %ymm4, sums.2+32(%rcx,%rcx)
  addq $32, %rcx
  vmovdqa %ymm2, mxs.0-32(%rcx)
  cmpq $2016, %rcx
  jne .L4
  movzwl a.3+2016(%rip), %ecx
  movl $a.3+2016, %edx
  xorl %edi, %edi
  movl %ecx, %esi
.L5:
  movswl (%rdx), %r8d
  addl %r8d, %edi
  cmpw %r8w, %cx
  cmovg %r8d, %ecx
  cmpw %r8w, %si
  cmovl %r8d, %esi
  addq $2, %rdx
  cmpq $a.3+2048, %rdx
  jne .L5
  movl %edi, sums.2+4032(%rip)
  xorl %eax, %eax
  movw %cx, mns.1+2016(%rip)
  movw %si, mxs.0+2016(%rip)
  xorl %esi, %esi
.L6:
  movq %rsi, %rcx
  movl sums.2(%rax,%rax), %edx
  addq $2, %rax
  salq $5, %rcx
  subq %rsi, %rcx
  addq %rcx, %rdx
  movq %rdx, %rcx
  salq $5, %rcx
  subq %rdx, %rcx
  movswl mns.1-2(%rax), %edx
  addl $1000, %edx
  addq %rcx, %rdx
  movq %rdx, %rcx
  salq $5, %rcx
  subq %rdx, %rcx
  movswl mxs.0-2(%rax), %edx
  leal 1000(%rdx), %esi
  addq %rcx, %rsi
  cmpq $2018, %rax
  jne .L6
  xorl %eax, %eax
  movl $.LC8, %edi
  vzeroupper
  call printf
  xorl %eax, %eax
  leave
  ret
.LC0:
