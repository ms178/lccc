.LCPI0_0:
  .long 0
  .long 1
  .long 2
  .long 3
  .long 4
  .long 5
  .long 6
  .long 7
.LCPI0_1:
  .long 15
.LCPI0_2:
  .long 8
.LCPI0_3:
  .long 0x3e800000
.LCPI0_4:
  .long 64
.LCPI0_5:
  .quad 0x41224f8000000000
.LCPI0_6:
  .quad 0x4122ebc000000000
main:
  pushq %rbx
  subq $16, %rsp
  movl $1000, %ebx
  cmpl $2, %edi
  jl .LBB0_2
  movq 8(%rsi), %rdi
  xorl %esi, %esi
  movl $10, %edx
  callq strtol@PLT
  movq %rax, %rbx
.LBB0_2:
  vmovaps .LCPI0_0(%rip), %ymm0
  xorl %eax, %eax
  vbroadcastss .LCPI0_1(%rip), %ymm1
  vbroadcastss .LCPI0_2(%rip), %ymm2
  vbroadcastss .LCPI0_3(%rip), %ymm3
  leaq input(%rip), %rcx
  vpbroadcastd .LCPI0_4(%rip), %ymm4
.LBB0_3:
  vandps %ymm1, %ymm0, %ymm5
  vxorps %ymm2, %ymm5, %ymm6
  vcvtdq2ps %ymm5, %ymm5
  vcvtdq2ps %ymm6, %ymm6
  vmulps %ymm3, %ymm5, %ymm5
  vmulps %ymm3, %ymm6, %ymm6
  vmovups %ymm5, (%rcx,%rax,4)
  vmovups %ymm6, 32(%rcx,%rax,4)
  vmovups %ymm5, 64(%rcx,%rax,4)
  vmovups %ymm6, 96(%rcx,%rax,4)
  vmovups %ymm5, 128(%rcx,%rax,4)
  vmovups %ymm6, 160(%rcx,%rax,4)
  vmovups %ymm5, 192(%rcx,%rax,4)
  vmovups %ymm6, 224(%rcx,%rax,4)
  addq $64, %rax
  vpaddd %ymm4, %ymm0, %ymm0
  cmpq $65536, %rax
  jne .LBB0_3
  testl %ebx, %ebx
  jle .LBB0_5
.LBB0_8:
  vzeroupper
  callq stencil5
  decl %ebx
  jne .LBB0_8
.LBB0_5:
  vxorps %xmm0, %xmm0, %xmm0
  xorl %eax, %eax
  leaq output(%rip), %rcx
.LBB0_6:
  vmovss (%rcx,%rax,4), %xmm2
  vmovss 4(%rcx,%rax,4), %xmm1
  vcvtss2sd %xmm2, %xmm2, %xmm2
  vcvtss2sd %xmm1, %xmm1, %xmm1
  vaddsd %xmm2, %xmm0, %xmm0
  vmovss 8(%rcx,%rax,4), %xmm2
  vcvtss2sd %xmm2, %xmm2, %xmm2
  vaddsd %xmm1, %xmm0, %xmm0
  vmovss 12(%rcx,%rax,4), %xmm1
  vcvtss2sd %xmm1, %xmm1, %xmm1
  vaddsd %xmm2, %xmm0, %xmm0
  vmovss 16(%rcx,%rax,4), %xmm2
  vcvtss2sd %xmm2, %xmm2, %xmm2
  vaddsd %xmm1, %xmm0, %xmm0
  vmovss 20(%rcx,%rax,4), %xmm1
  vcvtss2sd %xmm1, %xmm1, %xmm1
  vaddsd %xmm2, %xmm0, %xmm0
  vmovss 24(%rcx,%rax,4), %xmm2
  vcvtss2sd %xmm2, %xmm2, %xmm2
  vaddsd %xmm1, %xmm0, %xmm0
  vmovss 28(%rcx,%rax,4), %xmm1
  vcvtss2sd %xmm1, %xmm1, %xmm1
  vaddsd %xmm2, %xmm0, %xmm0
  vaddsd %xmm1, %xmm0, %xmm0
  addq $8, %rax
  cmpq $65536, %rax
  jne .LBB0_6
  leaq .L.str(%rip), %rdi
  movb $1, %al
  vmovsd %xmm0, 8(%rsp)
  vzeroupper
  callq printf@PLT
  vmovsd .LCPI0_5(%rip), %xmm0
  vmovsd 8(%rsp), %xmm2
  vcmpnltsd .LCPI0_6(%rip), %xmm2, %xmm1
  vcmpnltsd %xmm2, %xmm0, %xmm0
  vorpd %xmm1, %xmm0, %xmm0
  vmovd %xmm0, %eax
  andl $1, %eax
  addq $16, %rsp
  popq %rbx
  retq

stencil5:
  vbroadcastss input+8(%rip), %ymm0
  xorl %edx, %edx
  leaq input(%rip), %rax
  leaq output(%rip), %rcx
.LBB1_1:
  vmovups 64(%rax,%rdx,4), %ymm1
  vmovups 96(%rax,%rdx,4), %ymm2
  vmovups (%rax,%rdx,4), %ymm3
  vmovups 12(%rax,%rdx,4), %ymm4
  vmovups 32(%rax,%rdx,4), %ymm5
  vmovups 44(%rax,%rdx,4), %ymm6
  vaddps 4(%rax,%rdx,4), %ymm3, %ymm3
  vaddps 36(%rax,%rdx,4), %ymm5, %ymm5
  vaddps 68(%rax,%rdx,4), %ymm1, %ymm1
  vaddps 100(%rax,%rdx,4), %ymm2, %ymm2
  vmovups 76(%rax,%rdx,4), %ymm7
  vperm2f128 $33, %ymm4, %ymm0, %ymm8
  vmovups 108(%rax,%rdx,4), %ymm0
  vshufps $3, %ymm4, %ymm8, %ymm8
  vshufps $152, %ymm4, %ymm8, %ymm8
  vaddps %ymm3, %ymm8, %ymm3
  vperm2f128 $33, %ymm6, %ymm4, %ymm8
  vshufps $3, %ymm6, %ymm8, %ymm8
  vshufps $152, %ymm6, %ymm8, %ymm8
  vaddps %ymm5, %ymm8, %ymm5
  vperm2f128 $33, %ymm7, %ymm6, %ymm8
  vshufps $3, %ymm7, %ymm8, %ymm8
  vshufps $152, %ymm7, %ymm8, %ymm8
  vaddps %ymm1, %ymm8, %ymm1
  vperm2f128 $33, %ymm0, %ymm7, %ymm8
  vshufps $3, %ymm0, %ymm8, %ymm8
  vshufps $152, %ymm0, %ymm8, %ymm8
  vaddps %ymm2, %ymm8, %ymm2
  vaddps %ymm4, %ymm3, %ymm3
  vaddps %ymm6, %ymm5, %ymm4
  vaddps %ymm7, %ymm1, %ymm1
  vaddps %ymm0, %ymm2, %ymm2
  vaddps 16(%rax,%rdx,4), %ymm3, %ymm3
  vaddps 48(%rax,%rdx,4), %ymm4, %ymm4
  vaddps 80(%rax,%rdx,4), %ymm1, %ymm1
  vaddps 112(%rax,%rdx,4), %ymm2, %ymm2
  vmovups %ymm3, 8(%rcx,%rdx,4)
  vmovups %ymm4, 40(%rcx,%rdx,4)
  vmovups %ymm1, 72(%rcx,%rdx,4)
  vmovups %ymm2, 104(%rcx,%rdx,4)
  addq $32, %rdx
  cmpq $65504, %rdx
  jne .LBB1_1
  vextractf128 $1, %ymm0, %xmm0
  vshufps $255, %xmm0, %xmm0, %xmm0
  xorl %edx, %edx
.LBB1_3:
  vmovss 262020(%rax,%rdx,4), %xmm1
  vmovss 262028(%rax,%rdx,4), %xmm2
  vaddss 262016(%rax,%rdx,4), %xmm1, %xmm3
  vaddss %xmm0, %xmm3, %xmm0
  vaddss %xmm2, %xmm0, %xmm3
  vmovss 262032(%rax,%rdx,4), %xmm0
  vaddss %xmm0, %xmm3, %xmm3
  vmovss %xmm3, 262024(%rcx,%rdx,4)
  vaddss 262024(%rax,%rdx,4), %xmm1, %xmm1
  vaddss %xmm2, %xmm1, %xmm1
  vaddss %xmm0, %xmm1, %xmm1
  vaddss 262036(%rax,%rdx,4), %xmm1, %xmm1
  vmovss %xmm1, 262028(%rcx,%rdx,4)
  addq $2, %rdx
  cmpq $28, %rdx
  jne .LBB1_3
  vzeroupper
  retq

.L.str:
  .asciz "%.0f\n"

