.LC8:
main:
  movl $717, %ecx
  pushq %rbp
  movl $a.3, %eax
  movl $a.3+2048, %edx
  vmovd %ecx, %xmm7
  movl $41, %ecx
  vmovdqa .LC0(%rip), %ymm4
  vpxor %xmm8, %xmm8, %xmm8
  vmovd %ecx, %xmm6
  movl $8, %ecx
  vpbroadcastd %xmm7, %ymm7
  vmovd %ecx, %xmm11
  movl $1098962147, %ecx
  movq %rsp, %rbp
  andq $-32, %rsp
  vmovd %ecx, %xmm3
  movl $2001, %ecx
  vpbroadcastd %xmm6, %ymm6
  vmovd %ecx, %xmm5
  movl $-65471464, %ecx
  vpbroadcastd %xmm11, %ymm11
  vmovd %ecx, %xmm10
  movl $16, %ecx
  vpbroadcastd %xmm3, %ymm3
  vmovd %ecx, %xmm9
  vpbroadcastd %xmm5, %ymm5
  vpbroadcastd %xmm10, %ymm10
  vpbroadcastd %xmm9, %ymm9
.L2:
  vpmulld %ymm7, %ymm4, %ymm0
  vpaddd %ymm11, %ymm4, %ymm1
  addq $32, %rax
  vpmulld %ymm7, %ymm1, %ymm1
  vpaddd %ymm9, %ymm4, %ymm4
  vpaddd %ymm6, %ymm0, %ymm0
  vpmuldq %ymm3, %ymm0, %ymm12
  vpsrlq $32, %ymm0, %ymm2
  vpaddd %ymm6, %ymm1, %ymm1
  vpmuldq %ymm3, %ymm2, %ymm2
  vpshufd $245, %ymm12, %ymm12
  vpblendd $85, %ymm12, %ymm2, %ymm2
  vpmuldq %ymm3, %ymm1, %ymm12
  vpsrad $9, %ymm2, %ymm2
  vpmulld %ymm5, %ymm2, %ymm2
  vpshufd $245, %ymm12, %ymm12
  vpsubd %ymm2, %ymm0, %ymm0
  vpsrlq $32, %ymm1, %ymm2
  vpmuldq %ymm3, %ymm2, %ymm2
  vpblendw $170, %ymm8, %ymm0, %ymm0
  vpblendd $85, %ymm12, %ymm2, %ymm2
  vpsrad $9, %ymm2, %ymm2
  vpmulld %ymm5, %ymm2, %ymm2
  vpsubd %ymm2, %ymm1, %ymm1
  vpblendw $170, %ymm8, %ymm1, %ymm1
  vpackusdw %ymm1, %ymm0, %ymm0
  vpermq $216, %ymm0, %ymm0
  vpaddw %ymm0, %ymm10, %ymm0
  vmovdqa %ymm0, -32(%rax)
  cmpq %rax, %rdx
  jne .L2
  xorl %eax, %eax
.L3:
  vmovdqa a.3(%rax), %ymm2
  vmovdqu a.3+2(%rax), %ymm6
  vmovdqu a.3+4(%rax), %ymm1
  vmovdqu a.3+6(%rax), %ymm14
  vpmovsxwd %xmm6, %ymm15
  vpmovsxwd %xmm2, %ymm0
  vmovdqu a.3+8(%rax), %ymm3
  vmovdqu a.3+10(%rax), %ymm10
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd %xmm1, %ymm15
  vmovdqu a.3+12(%rax), %ymm5
  vmovdqu a.3+14(%rax), %ymm13
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd %xmm14, %ymm15
  vmovdqu a.3+16(%rax), %ymm9
  vmovdqu a.3+18(%rax), %ymm4
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd %xmm3, %ymm15
  vmovdqu a.3+20(%rax), %ymm12
  vmovdqu a.3+22(%rax), %ymm8
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd %xmm10, %ymm15
  vmovdqu a.3+26(%rax), %ymm11
  vmovdqu a.3+28(%rax), %ymm7
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd %xmm5, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd %xmm13, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd %xmm9, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd %xmm4, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd %xmm12, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd %xmm8, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd a.3+24(%rax), %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd %xmm11, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd %xmm7, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vpmovsxwd a.3+30(%rax), %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vextracti128 $0x1, %ymm6, %xmm15
  vmovdqa %ymm0, sums.2(%rax,%rax)
  vextracti128 $0x1, %ymm2, %xmm0
  vpmovsxwd %xmm15, %ymm15
  vpmovsxwd %xmm0, %ymm0
  vpaddd %ymm15, %ymm0, %ymm0
  vextracti128 $0x1, %ymm1, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vextracti128 $0x1, %ymm14, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vextracti128 $0x1, %ymm3, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vextracti128 $0x1, %ymm10, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vextracti128 $0x1, %ymm5, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vextracti128 $0x1, %ymm13, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vextracti128 $0x1, %ymm9, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vextracti128 $0x1, %ymm4, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vextracti128 $0x1, %ymm12, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vextracti128 $0x1, %ymm8, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vmovdqu a.3+24(%rax), %ymm15
  vextracti128 $0x1, %ymm15, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vextracti128 $0x1, %ymm11, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vextracti128 $0x1, %ymm7, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vmovdqu a.3+30(%rax), %ymm15
  vextracti128 $0x1, %ymm15, %xmm15
  vpmovsxwd %xmm15, %ymm15
  vpaddd %ymm15, %ymm0, %ymm0
  vpminsw %ymm10, %ymm3, %ymm15
  vmovdqa %ymm0, sums.2+32(%rax,%rax)
  vpminsw %ymm14, %ymm1, %ymm0
  vpminsw %ymm9, %ymm15, %ymm15
  addq $32, %rax
  vpminsw %ymm13, %ymm0, %ymm0
  vpminsw %ymm8, %ymm15, %ymm15
  vpminsw %ymm12, %ymm0, %ymm0
  vpminsw %ymm7, %ymm15, %ymm15
  vpminsw %ymm11, %ymm0, %ymm0
  vpminsw %ymm15, %ymm0, %ymm0
  vpminsw %ymm6, %ymm2, %ymm15
  vpminsw %ymm5, %ymm15, %ymm15
  vpminsw %ymm4, %ymm15, %ymm15
  vpminsw a.3-8(%rax), %ymm15, %ymm15
  vpminsw a.3-2(%rax), %ymm15, %ymm15
  vpminsw %ymm15, %ymm0, %ymm0
  vmovdqa %ymm0, mns.1-32(%rax)
  vpmaxsw %ymm14, %ymm1, %ymm0
  vpmaxsw %ymm10, %ymm3, %ymm1
  vpmaxsw %ymm13, %ymm0, %ymm0
  vpmaxsw %ymm9, %ymm1, %ymm1
  vpmaxsw %ymm12, %ymm0, %ymm0
  vpmaxsw %ymm8, %ymm1, %ymm1
  vpmaxsw %ymm7, %ymm1, %ymm1
  vpmaxsw %ymm11, %ymm0, %ymm0
  vpmaxsw %ymm1, %ymm0, %ymm0
  vpmaxsw %ymm6, %ymm2, %ymm1
  vpmaxsw %ymm5, %ymm1, %ymm1
  vpmaxsw %ymm4, %ymm1, %ymm1
  vpmaxsw a.3-8(%rax), %ymm1, %ymm1
  vpmaxsw a.3-2(%rax), %ymm1, %ymm1
  vpmaxsw %ymm1, %ymm0, %ymm0
  vmovdqa %ymm0, mxs.0-32(%rax)
  cmpq $2016, %rax
  jne .L3
  vmovdqa a.3+2016(%rip), %ymm2
  xorl %eax, %eax
  xorl %esi, %esi
  vextracti128 $0x1, %ymm2, %xmm0
  vpmovsxwd %xmm2, %ymm1
  vpmovsxwd %xmm0, %ymm3
  vpaddd %ymm1, %ymm3, %ymm3
  vextracti128 $0x1, %ymm3, %xmm1
  vpaddd %xmm3, %xmm1, %xmm1
  vpsrldq $8, %xmm1, %xmm3
  vpaddd %xmm3, %xmm1, %xmm1
  vpsrldq $4, %xmm1, %xmm3
  vpaddd %xmm3, %xmm1, %xmm1
  vmovd %xmm1, sums.2+4032(%rip)
  vpminsw %xmm2, %xmm0, %xmm1
  vpmaxsw %xmm2, %xmm0, %xmm0
  vpsrldq $8, %xmm1, %xmm3
  vpminsw %xmm3, %xmm1, %xmm1
  vpsrldq $4, %xmm1, %xmm3
  vpminsw %xmm3, %xmm1, %xmm1
  vpsrldq $2, %xmm1, %xmm3
  vpminsw %xmm3, %xmm1, %xmm1
  vpextrw $0, %xmm1, mns.1+2016(%rip)
  vpsrldq $8, %xmm0, %xmm1
  vpmaxsw %xmm1, %xmm0, %xmm0
  vpsrldq $4, %xmm0, %xmm1
  vpmaxsw %xmm1, %xmm0, %xmm0
  vpsrldq $2, %xmm0, %xmm1
  vpmaxsw %xmm1, %xmm0, %xmm0
  vpextrw $0, %xmm0, mxs.0+2016(%rip)
.L4:
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
  jne .L4
  xorl %eax, %eax
  movl $.LC8, %edi
  vzeroupper
  call printf
  xorl %eax, %eax
  leave
  ret
.LC0:
