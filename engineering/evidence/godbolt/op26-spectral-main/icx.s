main:
  subq $48008, %rsp
  vstmxcsr 16000(%rsp)
  orl $32832, 16000(%rsp)
  vldmxcsr 16000(%rsp)
  movq $-40, %rax
  vbroadcastsd .LCPI0_0(%rip), %ymm0
.LBB0_1:
  vmovups %ymm0, 320(%rsp,%rax,8)
  vmovups %ymm0, 352(%rsp,%rax,8)
  vmovups %ymm0, 384(%rsp,%rax,8)
  vmovups %ymm0, 416(%rsp,%rax,8)
  vmovups %ymm0, 448(%rsp,%rax,8)
  vmovups %ymm0, 480(%rsp,%rax,8)
  vmovups %ymm0, 512(%rsp,%rax,8)
  vmovups %ymm0, 544(%rsp,%rax,8)
  vmovups %ymm0, 576(%rsp,%rax,8)
  vmovups %ymm0, 608(%rsp,%rax,8)
  addq $40, %rax
  cmpq $1960, %rax
  jb .LBB0_1
  xorl %eax, %eax
  vmovdqu .LCPI0_1(%rip), %xmm0
  vmovdqu .LCPI0_2(%rip), %xmm1
  vpcmpeqd %xmm2, %xmm2, %xmm2
.LBB0_3:
  movl $4, %ecx
  xorl %edx, %edx
.LBB0_4:
  vmovd %edx, %xmm3
  vpbroadcastd %xmm3, %xmm4
  vpxor %xmm3, %xmm3, %xmm3
  movq $-4, %rsi
  movl %ecx, %edi
.LBB0_5:
  leal -4(%rdi), %r8d
  vmovd %r8d, %xmm5
  vpbroadcastd %xmm5, %xmm5
  vpaddd %xmm0, %xmm5, %xmm6
  vpaddd %xmm1, %xmm5, %xmm5
  vpmulld %xmm5, %xmm6, %xmm5
  vpsrld $1, %xmm5, %xmm5
  vmovd %edi, %xmm6
  vpbroadcastd %xmm6, %xmm6
  vpaddd %xmm0, %xmm6, %xmm7
  vpaddd %xmm4, %xmm5, %xmm5
  vpaddd %xmm1, %xmm6, %xmm6
  vpmulld %xmm6, %xmm7, %xmm6
  vpsubd %xmm2, %xmm5, %xmm5
  vmovupd 32(%rsp,%rsi,8), %ymm7
  vpsrld $1, %xmm6, %xmm6
  vcvtdq2pd %xmm5, %ymm5
  vpaddd %xmm4, %xmm6, %xmm6
  vpsubd %xmm2, %xmm6, %xmm6
  vcvtdq2pd %xmm6, %ymm6
  vmovupd 64(%rsp,%rsi,8), %ymm8
  vdivpd %ymm5, %ymm7, %ymm5
  vdivpd %ymm6, %ymm8, %ymm6
  vaddpd %ymm3, %ymm5, %ymm3
  vaddpd %ymm3, %ymm6, %ymm3
  addl $8, %edi
  addq $8, %rsi
  cmpq $1996, %rsi
  jb .LBB0_5
  vextractf128 $1, %ymm3, %xmm4
  vaddpd %xmm4, %xmm3, %xmm3
  vshufpd $1, %xmm3, %xmm3, %xmm4
  vaddsd %xmm4, %xmm3, %xmm3
  vmovsd %xmm3, 16000(%rsp,%rdx,8)
  leaq 1(%rdx), %rsi
  incl %ecx
  cmpq $1999, %rdx
  movq %rsi, %rdx
  jne .LBB0_4
  movl $4, %ecx
  xorl %edx, %edx
.LBB0_8:
  vxorpd %xmm3, %xmm3, %xmm3
  xorl %esi, %esi
.LBB0_9:
  leal (%rdx,%rsi), %edi
  vmovd %edi, %xmm4
  vpbroadcastd %xmm4, %xmm4
  vpaddd %xmm0, %xmm4, %xmm5
  vpaddd %xmm1, %xmm4, %xmm4
  vmovd %esi, %xmm6
  vpbroadcastd %xmm6, %xmm6
  vpmulld %xmm4, %xmm5, %xmm4
  vpaddd %xmm0, %xmm6, %xmm5
  leal (%rcx,%rsi), %edi
  vmovd %edi, %xmm6
  vpsrld $1, %xmm4, %xmm4
  vpbroadcastd %xmm6, %xmm6
  vpaddd %xmm0, %xmm6, %xmm7
  vpaddd %xmm1, %xmm6, %xmm6
  vpaddd %xmm5, %xmm4, %xmm4
  vpmulld %xmm6, %xmm7, %xmm5
  leaq 4(%rsi), %rdi
  vpsrld $1, %xmm5, %xmm5
  vmovd %edi, %xmm6
  vpbroadcastd %xmm6, %xmm6
  vcvtdq2pd %xmm4, %ymm4
  vpaddd %xmm0, %xmm6, %xmm6
  vpaddd %xmm6, %xmm5, %xmm5
  vcvtdq2pd %xmm5, %ymm5
  vmovupd 16000(%rsp,%rsi,8), %ymm6
  vdivpd %ymm4, %ymm6, %ymm4
  vmovupd 16032(%rsp,%rsi,8), %ymm6
  vdivpd %ymm5, %ymm6, %ymm5
  vaddpd %ymm3, %ymm4, %ymm3
  vaddpd %ymm3, %ymm5, %ymm3
  addq $8, %rsi
  cmpq $1996, %rdi
  jb .LBB0_9
  vextractf128 $1, %ymm3, %xmm4
  vaddpd %xmm4, %xmm3, %xmm3
  vshufpd $1, %xmm3, %xmm3, %xmm4
  vaddsd %xmm4, %xmm3, %xmm3
  vmovsd %xmm3, 32000(%rsp,%rdx,8)
  leaq 1(%rdx), %rsi
  incq %rcx
  cmpq $1999, %rdx
  movq %rsi, %rdx
  jne .LBB0_8
  movl $4, %ecx
  xorl %edx, %edx
.LBB0_12:
  vmovd %edx, %xmm3
  vpbroadcastd %xmm3, %xmm4
  vpxor %xmm3, %xmm3, %xmm3
  movq $-4, %rsi
  movl %ecx, %edi
.LBB0_13:
  leal -4(%rdi), %r8d
  vmovd %r8d, %xmm5
  vpbroadcastd %xmm5, %xmm5
  vpaddd %xmm0, %xmm5, %xmm6
  vpaddd %xmm1, %xmm5, %xmm5
  vpmulld %xmm5, %xmm6, %xmm5
  vpsrld $1, %xmm5, %xmm5
  vmovd %edi, %xmm6
  vpbroadcastd %xmm6, %xmm6
  vpaddd %xmm0, %xmm6, %xmm7
  vpaddd %xmm4, %xmm5, %xmm5
  vpaddd %xmm1, %xmm6, %xmm6
  vpmulld %xmm6, %xmm7, %xmm6
  vpsubd %xmm2, %xmm5, %xmm5
  vmovupd 32032(%rsp,%rsi,8), %ymm7
  vpsrld $1, %xmm6, %xmm6
  vcvtdq2pd %xmm5, %ymm5
  vpaddd %xmm4, %xmm6, %xmm6
  vpsubd %xmm2, %xmm6, %xmm6
  vcvtdq2pd %xmm6, %ymm6
  vmovupd 32064(%rsp,%rsi,8), %ymm8
  vdivpd %ymm5, %ymm7, %ymm5
  vdivpd %ymm6, %ymm8, %ymm6
  vaddpd %ymm3, %ymm5, %ymm3
  vaddpd %ymm3, %ymm6, %ymm3
  addl $8, %edi
  addq $8, %rsi
  cmpq $1996, %rsi
  jb .LBB0_13
  vextractf128 $1, %ymm3, %xmm4
  vaddpd %xmm4, %xmm3, %xmm3
  vshufpd $1, %xmm3, %xmm3, %xmm4
  vaddsd %xmm4, %xmm3, %xmm3
  vmovsd %xmm3, 16000(%rsp,%rdx,8)
  leaq 1(%rdx), %rsi
  incl %ecx
  cmpq $1999, %rdx
  movq %rsi, %rdx
  jne .LBB0_12
  movl $4, %ecx
  xorl %edx, %edx
.LBB0_16:
  vxorpd %xmm3, %xmm3, %xmm3
  xorl %esi, %esi
.LBB0_17:
  leal (%rdx,%rsi), %edi
  vmovd %edi, %xmm4
  vpbroadcastd %xmm4, %xmm4
  vpaddd %xmm0, %xmm4, %xmm5
  vpaddd %xmm1, %xmm4, %xmm4
  vmovd %esi, %xmm6
  vpbroadcastd %xmm6, %xmm6
  vpmulld %xmm4, %xmm5, %xmm4
  vpaddd %xmm0, %xmm6, %xmm5
  leal (%rcx,%rsi), %edi
  vmovd %edi, %xmm6
  vpsrld $1, %xmm4, %xmm4
  vpbroadcastd %xmm6, %xmm6
  vpaddd %xmm0, %xmm6, %xmm7
  vpaddd %xmm1, %xmm6, %xmm6
  vpaddd %xmm5, %xmm4, %xmm4
  vpmulld %xmm6, %xmm7, %xmm5
  leaq 4(%rsi), %rdi
  vpsrld $1, %xmm5, %xmm5
  vmovd %edi, %xmm6
  vpbroadcastd %xmm6, %xmm6
  vcvtdq2pd %xmm4, %ymm4
  vpaddd %xmm0, %xmm6, %xmm6
  vpaddd %xmm6, %xmm5, %xmm5
  vcvtdq2pd %xmm5, %ymm5
  vmovupd 16000(%rsp,%rsi,8), %ymm6
  vdivpd %ymm4, %ymm6, %ymm4
  vmovupd 16032(%rsp,%rsi,8), %ymm6
  vdivpd %ymm5, %ymm6, %ymm5
  vaddpd %ymm3, %ymm4, %ymm3
  vaddpd %ymm3, %ymm5, %ymm3
  addq $8, %rsi
  cmpq $1996, %rdi
  jb .LBB0_17
  vextractf128 $1, %ymm3, %xmm4
  vaddpd %xmm4, %xmm3, %xmm3
  vshufpd $1, %xmm3, %xmm3, %xmm4
  vaddsd %xmm4, %xmm3, %xmm3
  vmovsd %xmm3, (%rsp,%rdx,8)
  leaq 1(%rdx), %rsi
  incq %rcx
  cmpq $1999, %rdx
  movq %rsi, %rdx
  jne .LBB0_16
  leal 1(%rax), %ecx
  cmpl $9, %eax
  movl %ecx, %eax
  jne .LBB0_3
  vpxor %xmm0, %xmm0, %xmm0
  movq $-20, %rax
  vpxor %xmm1, %xmm1, %xmm1
.LBB0_21:
  vmovupd 32160(%rsp,%rax,8), %ymm2
  vmovupd 32192(%rsp,%rax,8), %ymm3
  vmovupd 32224(%rsp,%rax,8), %ymm4
  vmovupd 32256(%rsp,%rax,8), %ymm5
  vfmadd231pd 160(%rsp,%rax,8), %ymm2, %ymm1
  vfmadd231pd %ymm2, %ymm2, %ymm0
  vfmadd231pd 192(%rsp,%rax,8), %ymm3, %ymm1
  vfmadd231pd %ymm3, %ymm3, %ymm0
  vfmadd231pd 224(%rsp,%rax,8), %ymm4, %ymm1
  vfmadd231pd 256(%rsp,%rax,8), %ymm5, %ymm1
  vfmadd231pd %ymm4, %ymm4, %ymm0
  vmovupd 32288(%rsp,%rax,8), %ymm2
  vfmadd231pd 288(%rsp,%rax,8), %ymm2, %ymm1
  vfmadd231pd %ymm5, %ymm5, %ymm0
  vfmadd231pd %ymm2, %ymm2, %ymm0
  addq $20, %rax
  cmpq $1980, %rax
  jb .LBB0_21
  vextractf128 $1, %ymm1, %xmm2
  vaddpd %xmm2, %xmm1, %xmm1
  vshufpd $1, %xmm1, %xmm1, %xmm2
  vaddsd %xmm2, %xmm1, %xmm1
  vextractf128 $1, %ymm0, %xmm2
  vaddpd %xmm2, %xmm0, %xmm0
  vshufpd $1, %xmm0, %xmm0, %xmm2
  vaddsd %xmm2, %xmm0, %xmm0
  vdivsd %xmm0, %xmm1, %xmm0
  vsqrtsd %xmm0, %xmm0, %xmm0
  movl $.L.str, %edi
  movb $1, %al
  vzeroupper
  callq printf
  xorl %eax, %eax
  addq $48008, %rsp
  retq

.L.str:
  .asciz "%.9f\n"
